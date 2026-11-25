//! Session lifetime rules. Pure functions so they can be unit tested without a database.

use chrono::{DateTime, Duration, Utc};

use super::client::Platform;

/// A session can never outlive this, however often it is refreshed.
pub const ABSOLUTE_DAYS: i64 = 365;

/// How long a previous refresh token is still accepted after rotation. Covers a refresh whose
/// response never reached the client; after this a replayed old token revokes the session.
pub const ROTATION_GRACE_SECONDS: i64 = 30;

/// Idle lifetime: every refresh pushes expiry this far out. Apps get longer so that "log in
/// once" holds for anyone who opens the app now and then.
pub fn idle_days(platform: Platform) -> i64 {
    match platform {
        Platform::Web => 14,
        Platform::Desktop | Platform::Mobile => 90,
    }
}

pub fn next_expiry(
    now: DateTime<Utc>,
    platform: Platform,
    absolute_expires_at: DateTime<Utc>,
) -> DateTime<Utc> {
    (now + Duration::days(idle_days(platform))).min(absolute_expires_at)
}

pub fn within_rotation_grace(rotated_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    rotated_at.is_some_and(|at| now >= at && now - at <= Duration::seconds(ROTATION_GRACE_SECONDS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_stay_logged_in_longer_than_the_web() {
        assert!(idle_days(Platform::Desktop) > idle_days(Platform::Web));
        assert_eq!(idle_days(Platform::Desktop), idle_days(Platform::Mobile));
    }

    #[test]
    fn expiry_slides_but_never_passes_the_absolute_cap() {
        let now = Utc::now();
        let cap = now + Duration::days(ABSOLUTE_DAYS);
        assert_eq!(next_expiry(now, Platform::Web, cap), now + Duration::days(14));
        assert_eq!(next_expiry(now, Platform::Desktop, cap), now + Duration::days(90));
        let late = cap - Duration::days(3);
        assert_eq!(next_expiry(late, Platform::Desktop, cap), cap);
    }

    #[test]
    fn rotation_grace_is_short_and_needs_a_rotation() {
        let now = Utc::now();
        assert!(!within_rotation_grace(None, now));
        assert!(within_rotation_grace(Some(now - Duration::seconds(5)), now));
        assert!(!within_rotation_grace(Some(now - Duration::seconds(31)), now));
        // A rotation "in the future" means the clock moved; never trust it.
        assert!(!within_rotation_grace(Some(now + Duration::seconds(5)), now));
    }
}
