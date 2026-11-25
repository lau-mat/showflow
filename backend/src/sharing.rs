//! Sharing a show: members with a role, and invites (a link, or a direct add for an existing
//! account). Only an owner manages access; anyone may leave. Every change to who can see what is
//! announced so connected clients refresh their show list straight away.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use protocol::show::MemberRole;
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::api_error::ApiError;
use crate::auth::{new_refresh_token, token_hash, AppState, AuthenticatedUser};
use crate::shows::{member_role, notify_access};

const DEFAULT_INVITE_DAYS: i64 = 7;
const MAX_INVITE_DAYS: i64 = 90;
const MAX_INVITE_USES: i32 = 100;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/shows/:id/members", get(list_members))
        .route("/shows/:id/members/:user_id", patch(change_role).delete(remove_member))
        .route("/shows/:id/invites", get(list_invites).post(create_invite))
        .route("/shows/:id/invites/:invite_id", delete(revoke_invite))
        .route("/invites/preview", post(preview_invite))
        .route("/invites/accept", post(accept_invite))
}

/// The roles that can be handed out. Ownership is not transferable (yet).
fn grantable(role: &str) -> Result<MemberRole, ApiError> {
    match MemberRole::parse(role) {
        Some(role @ (MemberRole::Editor | MemberRole::Commenter | MemberRole::Viewer)) => Ok(role),
        _ => Err(ApiError::BadRequest("Role must be editor, commenter or viewer".into())),
    }
}

async fn require_owner(conn: &mut PgConnection, show_id: Uuid, user_id: Uuid) -> Result<(), ApiError> {
    match member_role(conn, show_id, user_id).await? {
        None => Err(ApiError::NotFound),
        Some(role) if role.may_manage() => Ok(()),
        Some(_) => Err(ApiError::Forbidden("Only the owner can manage who has access")),
    }
}

async fn show_is_live(conn: &mut PgConnection, show_id: Uuid) -> Result<bool, ApiError> {
    let deleted: Option<bool> = sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM shows WHERE show_id = $1")
        .bind(show_id)
        .fetch_optional(conn)
        .await?;
    Ok(deleted == Some(false))
}

// ---------------------------------------------------------------------------------------------
// Members
// ---------------------------------------------------------------------------------------------

#[derive(Serialize)]
struct MemberView {
    user_id: Uuid,
    username: String,
    role: String,
    is_me: bool,
}

async fn list_members(
    State(state): State<AppState>,
    AuthenticatedUser(me): AuthenticatedUser,
    Path(show_id): Path<Uuid>,
) -> Result<Json<Vec<MemberView>>, ApiError> {
    let mut conn = state.pool.acquire().await?;
    member_role(&mut conn, show_id, me).await?.ok_or(ApiError::NotFound)?;
    let rows = sqlx::query(
        "SELECT m.user_id, u.user_name, m.role FROM show_members m JOIN users u ON u.user_id = m.user_id
         WHERE m.show_id = $1 ORDER BY (m.role = 'owner') DESC, u.user_name",
    )
    .bind(show_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| {
                let user_id: Uuid = row.get("user_id");
                MemberView { user_id, username: row.get("user_name"), role: row.get("role"), is_me: user_id == me }
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct RoleChange {
    role: String,
}

async fn change_role(
    State(state): State<AppState>,
    AuthenticatedUser(me): AuthenticatedUser,
    Path((show_id, user_id)): Path<(Uuid, Uuid)>,
    Json(change): Json<RoleChange>,
) -> Result<StatusCode, ApiError> {
    let role = grantable(&change.role)?;
    let mut tx = state.pool.begin().await?;
    require_owner(&mut tx, show_id, me).await?;
    // The owner's role is not changed through this endpoint.
    let changed = sqlx::query("UPDATE show_members SET role = $3 WHERE show_id = $1 AND user_id = $2 AND role <> 'owner'")
        .bind(show_id)
        .bind(user_id)
        .bind(role.as_str())
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if changed == 0 {
        return Err(ApiError::NotFound);
    }
    notify_access(&mut tx, Some(user_id)).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The owner removes someone, or a member removes themselves. An owner cannot leave: they
/// delete the show instead.
async fn remove_member(
    State(state): State<AppState>,
    AuthenticatedUser(me): AuthenticatedUser,
    Path((show_id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    let my_role = member_role(&mut tx, show_id, me).await?.ok_or(ApiError::NotFound)?;
    if user_id != me && !my_role.may_manage() {
        return Err(ApiError::Forbidden("Only the owner can remove other members"));
    }
    let removed = sqlx::query("DELETE FROM show_members WHERE show_id = $1 AND user_id = $2 AND role <> 'owner'")
        .bind(show_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        // Either not a member, or the owner.
        return if member_role(&mut tx, show_id, user_id).await?.is_some() {
            Err(ApiError::Forbidden("The owner cannot be removed; delete the show instead"))
        } else {
            Err(ApiError::NotFound)
        };
    }
    notify_access(&mut tx, Some(user_id)).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Invites
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct NewInvite {
    role: String,
    /// With an email, only that account can accept; an existing account is added right away.
    email: Option<String>,
    expires_in_days: Option<i64>,
    max_uses: Option<i32>,
}

#[derive(Serialize)]
struct InviteView {
    id: Uuid,
    role: String,
    email: Option<String>,
    expires_at: DateTime<Utc>,
    max_uses: i32,
    used_count: i32,
}

#[derive(Serialize)]
struct Invited {
    /// Set when the email belonged to an existing account that was added straight away.
    added: Option<AddedMember>,
    /// Set otherwise. `token` is shown once: only its hash is stored.
    invite: Option<InviteView>,
    token: Option<String>,
}

#[derive(Serialize)]
struct AddedMember {
    user_id: Uuid,
    username: String,
}

fn normalized_email(email: &str) -> Result<String, ApiError> {
    let email = email.trim().to_lowercase();
    if email.is_empty() || email.len() > 254 || !email.contains('@') || email.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(ApiError::BadRequest("A valid email address is required".into()));
    }
    Ok(email)
}

async fn create_invite(
    State(state): State<AppState>,
    AuthenticatedUser(me): AuthenticatedUser,
    Path(show_id): Path<Uuid>,
    Json(request): Json<NewInvite>,
) -> Result<(StatusCode, Json<Invited>), ApiError> {
    let role = grantable(&request.role)?;
    let email = request.email.as_deref().filter(|e| !e.trim().is_empty()).map(normalized_email).transpose()?;
    let days = request.expires_in_days.unwrap_or(DEFAULT_INVITE_DAYS);
    let max_uses = request.max_uses.unwrap_or(if email.is_some() { 1 } else { 10 });
    if !(1..=MAX_INVITE_DAYS).contains(&days) || !(1..=MAX_INVITE_USES).contains(&max_uses) {
        return Err(ApiError::BadRequest("Invite expiry or uses out of range".into()));
    }

    let mut tx = state.pool.begin().await?;
    require_owner(&mut tx, show_id, me).await?;
    if !show_is_live(&mut tx, show_id).await? {
        return Err(ApiError::Gone("This show has been deleted"));
    }

    // An account that already exists is added directly: no link to pass around.
    if let Some(email) = &email {
        let existing = sqlx::query("SELECT user_id, user_name FROM users WHERE lower(user_email) = $1 AND user_active IS TRUE")
            .bind(email)
            .fetch_optional(&mut *tx)
            .await?;
        if let Some(user) = existing {
            let user_id: Uuid = user.get("user_id");
            // Never downgrade or duplicate: an existing membership (including the owner's) stays.
            sqlx::query("INSERT INTO show_members (show_id, user_id, role, added_by) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING")
                .bind(show_id)
                .bind(user_id)
                .bind(role.as_str())
                .bind(me)
                .execute(&mut *tx)
                .await?;
            notify_access(&mut tx, Some(user_id)).await?;
            tx.commit().await?;
            return Ok((StatusCode::CREATED, Json(Invited {
                added: Some(AddedMember { user_id, username: user.get("user_name") }),
                invite: None,
                token: None,
            })));
        }
    }

    let token = new_refresh_token()?;
    let row = sqlx::query(
        "INSERT INTO show_invites (show_id, role, token_hash, email, created_by, expires_at, max_uses)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING invite_id, expires_at",
    )
    .bind(show_id)
    .bind(role.as_str())
    .bind(token_hash(&token))
    .bind(&email)
    .bind(me)
    .bind(Utc::now() + Duration::days(days))
    .bind(max_uses)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(Invited {
        added: None,
        invite: Some(InviteView {
            id: row.get("invite_id"),
            role: role.as_str().to_owned(),
            email,
            expires_at: row.get("expires_at"),
            max_uses,
            used_count: 0,
        }),
        token: Some(token),
    })))
}

async fn list_invites(
    State(state): State<AppState>,
    AuthenticatedUser(me): AuthenticatedUser,
    Path(show_id): Path<Uuid>,
) -> Result<Json<Vec<InviteView>>, ApiError> {
    let mut conn = state.pool.acquire().await?;
    require_owner(&mut conn, show_id, me).await?;
    let rows = sqlx::query(
        "SELECT invite_id, role, email, expires_at, max_uses, used_count FROM show_invites
         WHERE show_id = $1 AND revoked_at IS NULL AND expires_at > now() AND used_count < max_uses
         ORDER BY created_at DESC",
    )
    .bind(show_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| InviteView {
                id: row.get("invite_id"),
                role: row.get("role"),
                email: row.get("email"),
                expires_at: row.get("expires_at"),
                max_uses: row.get("max_uses"),
                used_count: row.get("used_count"),
            })
            .collect(),
    ))
}

async fn revoke_invite(
    State(state): State<AppState>,
    AuthenticatedUser(me): AuthenticatedUser,
    Path((show_id, invite_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let mut conn = state.pool.acquire().await?;
    require_owner(&mut conn, show_id, me).await?;
    sqlx::query("UPDATE show_invites SET revoked_at = COALESCE(revoked_at, now()) WHERE invite_id = $1 AND show_id = $2")
        .bind(invite_id)
        .bind(show_id)
        .execute(&mut *conn)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct TokenRequest {
    token: String,
}

#[derive(Serialize)]
struct InvitePreview {
    show_id: Uuid,
    show_name: String,
    role: String,
    invited_by: Option<String>,
    /// The caller already has access to this show.
    already_member: bool,
}

/// Why an invite cannot be used. All look the same to the caller: a link is either good or not.
const INVALID_INVITE: &str = "This invitation is no longer valid";

async fn find_usable_invite(conn: &mut PgConnection, token: &str, user_id: Uuid) -> Result<sqlx::postgres::PgRow, ApiError> {
    if token.len() != 43 {
        return Err(ApiError::Gone(INVALID_INVITE));
    }
    let row = sqlx::query(
        "SELECT i.invite_id, i.show_id, i.role, i.email, s.name, u.user_name AS invited_by,
                (SELECT email_ok FROM (SELECT lower(user_email) = lower(i.email) AS email_ok FROM users WHERE user_id = $2) e) AS email_matches
         FROM show_invites i
         JOIN shows s ON s.show_id = i.show_id AND s.deleted_at IS NULL
         LEFT JOIN users u ON u.user_id = i.created_by
         WHERE i.token_hash = $1 AND i.revoked_at IS NULL AND i.expires_at > now() AND i.used_count < i.max_uses",
    )
    .bind(token_hash(token))
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(ApiError::Gone(INVALID_INVITE))?;
    // An invitation for a particular email is useless to anyone else.
    let bound: Option<String> = row.get("email");
    if bound.is_some() && row.get::<Option<bool>, _>("email_matches") != Some(true) {
        return Err(ApiError::Gone(INVALID_INVITE));
    }
    Ok(row)
}

async fn preview_invite(
    State(state): State<AppState>,
    AuthenticatedUser(me): AuthenticatedUser,
    Json(request): Json<TokenRequest>,
) -> Result<Json<InvitePreview>, ApiError> {
    let mut conn = state.pool.acquire().await?;
    let invite = find_usable_invite(&mut conn, &request.token, me).await?;
    let show_id: Uuid = invite.get("show_id");
    Ok(Json(InvitePreview {
        show_id,
        show_name: invite.get("name"),
        role: invite.get("role"),
        invited_by: invite.get("invited_by"),
        already_member: member_role(&mut conn, show_id, me).await?.is_some(),
    }))
}

#[derive(Serialize)]
struct Joined {
    show_id: Uuid,
    role: String,
}

async fn accept_invite(
    State(state): State<AppState>,
    AuthenticatedUser(me): AuthenticatedUser,
    Json(request): Json<TokenRequest>,
) -> Result<Json<Joined>, ApiError> {
    let mut tx = state.pool.begin().await?;
    let invite = find_usable_invite(&mut tx, &request.token, me).await?;
    let (invite_id, show_id): (Uuid, Uuid) = (invite.get("invite_id"), invite.get("show_id"));

    if let Some(existing) = member_role(&mut tx, show_id, me).await? {
        // Already in: keep whatever role they have and do not use up the invitation.
        return Ok(Json(Joined { show_id, role: existing.as_str().to_owned() }));
    }
    // Claim a use atomically, so a single-use link cannot be accepted twice at once.
    let claimed = sqlx::query(
        "UPDATE show_invites SET used_count = used_count + 1
         WHERE invite_id = $1 AND used_count < max_uses AND revoked_at IS NULL AND expires_at > now()",
    )
    .bind(invite_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if claimed == 0 {
        return Err(ApiError::Gone(INVALID_INVITE));
    }
    let role: String = invite.get("role");
    sqlx::query("INSERT INTO show_members (show_id, user_id, role, added_by) VALUES ($1, $2, $3, (SELECT created_by FROM show_invites WHERE invite_id = $4))")
        .bind(show_id)
        .bind(me)
        .bind(&role)
        .bind(invite_id)
        .execute(&mut *tx)
        .await?;
    notify_access(&mut tx, Some(me)).await?;
    tx.commit().await?;
    Ok(Json(Joined { show_id, role }))
}

#[cfg(test)]
mod tests;
