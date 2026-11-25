//! The live-session wire protocol: what travels over the session's websocket between whoever runs
//! the show logic (the desktop app, later a cloud container) and the devices that joined.
//! `frontend/shared/js/session_types.ts` mirrors these.

use serde::{Deserialize, Serialize};

use crate::show::ShowDetails;

/// Bumped when a change would make an older client misbehave.
pub const PROTOCOL_VERSION: u32 = 2;

/// The two things a host guards, each with its own policy and code: getting in at all (`View`:
/// seeing the show) and running it (`Call`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AccessKind {
    View,
    Call,
}

/// How a device was let in, or came to be a show caller.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AccessMethod {
    Open,
    Code,
    Request,
    Account,
    /// The window on the computer that runs the session.
    Host,
}

/// Which ways through a gate the host enabled. `open` lets anyone in and makes the other
/// switches meaningless. With everything off only the host computer gets through.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccessPolicy {
    #[serde(default)]
    pub open: bool,
    #[serde(default)]
    pub code: bool,
    #[serde(default)]
    pub request: bool,
    #[serde(default)]
    pub account: bool,
}

impl AccessPolicy {
    /// `open` switches the other methods off, so a policy never says two contradicting things.
    pub fn normalized(self) -> Self {
        if self.open {
            Self { open: true, code: false, request: false, account: false }
        } else {
            self
        }
    }

    /// The one choice the app offers. For settings saved with several methods switched on: the
    /// first of open, code, account, request.
    pub fn single(self) -> Self {
        let none = Self::default();
        match self {
            Self { open: true, .. } => Self { open: true, ..none },
            Self { code: true, .. } => Self { code: true, ..none },
            Self { account: true, .. } => Self { account: true, ..none },
            _ => Self { request: self.request, ..none },
        }
    }

    pub fn allows(self, method: AccessMethod) -> bool {
        match method {
            AccessMethod::Open => self.open,
            AccessMethod::Code => self.code,
            AccessMethod::Request => self.request,
            AccessMethod::Account => self.account,
            AccessMethod::Host => true,
        }
    }
}

/// Who may do what in one show's sessions, as the host set it up.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AccessSettings {
    pub caller: AccessPolicy,
    pub caller_code: Option<String>,
    pub view: AccessPolicy,
    pub view_code: Option<String>,
}

/// Anyone on the network may watch; calling takes the host's approval.
impl Default for AccessSettings {
    fn default() -> Self {
        Self {
            caller: AccessPolicy { request: true, ..Default::default() },
            caller_code: None,
            view: AccessPolicy { open: true, ..Default::default() },
            view_code: None,
        }
    }
}

/// "Let me in", or "let me call the show"."""
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Claim {
    Open { name: String },
    Code { name: String, code: String },
    Request { name: String },
    /// A pass signed by the cloud plus proof that this device is the one it was issued to.
    Account { pass: String, proof: String },
    /// A token handed out by an earlier grant (a reload, a Wi-Fi drop).
    Resume { token: String },
    /// The host's own window; the secret is only readable inside the app.
    Host { token: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ClientMessage {
    JoinSession { user_name: String, role_id: Option<String> },
    GetShowData,
    GetSessionData,
    /// Asks to be let in. Until then a protected session sends nothing about the show.
    RequestView { claim: Claim },
    /// Asks for the caller role. A device must be in first, unless it presents a token.
    ClaimCaller { claim: Claim },
    ReleaseCaller,
    /// `from_cue` is the cue the caller was looking at (`None`: the show has not started). If the
    /// show already moved on, the press is ignored, so two callers cannot skip a cue together.
    NextCue { from_cue: Option<i64> },
    PreviousCue { from_cue: Option<i64> },
    JumpToCue { cue_number: i64 },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessState {
    Granted,
    /// Waiting for the host to approve.
    Pending,
    Denied,
    /// Was a caller and no longer is.
    Revoked,
    Released,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CallerInfo {
    pub name: String,
    pub method: AccessMethod,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct JoinedUser {
    pub client_id: String,
    pub client_name: Option<String>,
    pub client_role: Option<String>,
    pub is_caller: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub show_id: String,
    pub is_active: bool,
    pub current_cue: Option<i64>,
    pub show_started_at: Option<f64>,
    pub cue_started_at: Option<f64>,
    pub joined_users: Vec<JoinedUser>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ServerMessage {
    /// First message on every connection. `nonce` is fresh per connection (account proofs sign
    /// it). `policy` is for the caller role. With an open `view_policy` the device is already in.
    Welcome { protocol: u32, session_id: String, show_id: String, nonce: String, policy: AccessPolicy, view_policy: AccessPolicy },
    /// The host changed who may become caller, or who may join.
    Policy { policy: AccessPolicy, view_policy: AccessPolicy },
    /// Whether this device is in. Show content follows `granted` by itself.
    ViewStatus { state: AccessState, reason: Option<String>, resume_token: Option<String> },
    CallerStatus {
        state: AccessState,
        reason: Option<String>,
        /// Sent with `granted`: present it later to become caller again without asking.
        resume_token: Option<String>,
    },
    /// Who is calling right now.
    Callers { list: Vec<CallerInfo> },
    JoinSession { client_id: String, client_name: String, client_role: String },
    GetShowData { data: ShowDetails },
    GetSessionData { session: SessionSnapshot },
    CueChange {
        current_cue: i64,
        next_cue: Option<i64>,
        show_started_at: Option<f64>,
        cue_started_at: Option<f64>,
        /// The caller who moved the show.
        by: Option<String>,
    },
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn open_switches_the_other_methods_off() {
        let policy = AccessPolicy { open: true, code: true, request: true, account: true }.normalized();
        assert_eq!(policy, AccessPolicy { open: true, ..Default::default() });
        assert!(policy.allows(AccessMethod::Open) && !policy.allows(AccessMethod::Code));
        // The host's own window is always allowed, whatever the policy.
        assert!(AccessPolicy::default().allows(AccessMethod::Host));
        for method in [AccessMethod::Open, AccessMethod::Code, AccessMethod::Request, AccessMethod::Account] {
            assert!(!AccessPolicy::default().allows(method));
        }
    }

    #[test]
    fn settings_with_several_methods_come_down_to_one_choice() {
        let several = AccessPolicy { open: false, code: true, request: true, account: true };
        assert_eq!(several.single(), AccessPolicy { code: true, ..Default::default() });
        assert_eq!(AccessPolicy { request: true, ..Default::default() }.single(), AccessPolicy { request: true, ..Default::default() });
        assert_eq!(AccessPolicy::default().single(), AccessPolicy::default(), "only the host computer");
    }

    #[test]
    fn messages_have_the_shape_the_frontend_sends_and_reads() {
        let claim: ClientMessage = serde_json::from_value(json!({
            "command": "claim_caller", "claim": { "method": "code", "name": "Sam", "code": "abc123" }
        })).unwrap();
        assert_eq!(claim, ClientMessage::ClaimCaller { claim: Claim::Code { name: "Sam".into(), code: "abc123".into() } });
        let next: ClientMessage = serde_json::from_value(json!({"command": "next_cue", "from_cue": null})).unwrap();
        assert_eq!(next, ClientMessage::NextCue { from_cue: None });
        let status = serde_json::to_value(ServerMessage::CallerStatus {
            state: AccessState::Granted, reason: None, resume_token: Some("t".into()),
        }).unwrap();
        assert_eq!(status, json!({"command": "caller_status", "state": "granted", "reason": null, "resume_token": "t"}));
        let policy: AccessPolicy = serde_json::from_value(json!({"code": true})).unwrap();
        assert!(policy.code && !policy.open);
        let join: ClientMessage = serde_json::from_value(json!({"command": "request_view", "claim": {"method": "request", "name": "Sam"}})).unwrap();
        assert_eq!(join, ClientMessage::RequestView { claim: Claim::Request { name: "Sam".into() } });
        assert_eq!(serde_json::to_value(AccessKind::View).unwrap(), json!("view"));
    }

    #[test]
    fn by_default_anyone_watches_and_calling_is_approved_by_the_host() {
        let settings = AccessSettings::default();
        assert_eq!(settings.view, AccessPolicy { open: true, ..Default::default() });
        assert_eq!(settings.caller, AccessPolicy { request: true, ..Default::default() });
        assert!(settings.caller_code.is_none() && settings.view_code.is_none());
    }
}
