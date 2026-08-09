//! Auth orchestration — pure decision logic, no I/O.
//!
//! Every function here is a pure function of its inputs, so the security
//! rules (lockout, session rotation, token-reuse detection) are unit-testable
//! without a database. The API layer maps the decisions onto the real
//! repositories (users, sessions, failed_logins, audit_logs).
//!
//! The DB never makes policy decisions; it only stores facts.

use std::time::{Duration, SystemTime};

/// Account lifecycle status — mirrors the `users.status` CHECK constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountStatus {
    PendingVerification,
    Active,
    Suspended,
    Deleted,
}

impl AccountStatus {
    /// Parse from the stored TEXT value.
    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "pending_verification" => Some(Self::PendingVerification),
            "active" => Some(Self::Active),
            "suspended" => Some(Self::Suspended),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }

    /// May this account authenticate at all?
    pub fn can_authenticate(self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthPolicyError {
    #[error("account is not active")]
    AccountNotActive,
    #[error("account is locked out; try again later")]
    LockedOut,
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("refresh token has expired")]
    RefreshExpired,
    #[error("refresh token was reused — session family revoked")]
    TokenReuseDetected,
}

/// Lockout policy: N consecutive failures within a window locks the account.
///
/// Window slides: a failure older than `window` no longer counts. Once locked,
/// the account stays locked until the lockout expires (proportional backoff).
#[derive(Debug, Clone, Copy)]
pub struct LockoutPolicy {
    pub max_failures: u32,
    pub window: Duration,
    pub base_lockout: Duration,
}

impl LockoutPolicy {
    pub const fn new(max_failures: u32, window: Duration, base_lockout: Duration) -> Self {
        Self {
            max_failures,
            window,
            base_lockout,
        }
    }

    /// Decide whether a login attempt is currently allowed, given the count of
    /// recent failures (within `window`) and the time of the last failure.
    ///
    /// Returns the lockout remaining duration when the account is locked.
    pub fn evaluate(
        &self,
        recent_failures: u32,
        last_failure_at: SystemTime,
        now: SystemTime,
    ) -> Result<(), AuthPolicyError> {
        if recent_failures < self.max_failures {
            return Ok(());
        }
        // Lockout grows with the failure count past the threshold.
        let extra = recent_failures - self.max_failures;
        let lockout = self
            .base_lockout
            .checked_mul(2u32.saturating_pow(extra.min(6)))
            .unwrap_or(self.base_lockout);
        let unlock_at = last_failure_at + lockout;
        if now < unlock_at {
            return Err(AuthPolicyError::LockedOut);
        }
        Ok(())
    }
}

/// Decision for a refresh attempt against one stored session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshDecision {
    /// Presented token matches the live session — rotate it.
    Rotate,
    /// Presented token is an ancestor of the live session — token reuse.
    /// The whole session family must be revoked and the account flagged.
    ReuseDetected,
    /// Presented token matches no session we know about.
    Unknown,
}

/// Session rotation with reuse detection.
///
/// `presented_hash` is the SHA-256 of the presented token; `live_hash` is the
/// current session's stored hash; `ancestor_hashes` are the hashes of the
/// tokens this session chain rotated away from (older siblings). A token that
/// was already rotated out must never work again.
pub fn evaluate_refresh(
    presented_hash: &str,
    live_hash: &str,
    ancestor_hashes: &[&str],
) -> RefreshDecision {
    if presented_hash == live_hash {
        return RefreshDecision::Rotate;
    }
    if ancestor_hashes.contains(&presented_hash) {
        return RefreshDecision::ReuseDetected;
    }
    RefreshDecision::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn account_status_parses_and_gates() {
        assert_eq!(
            AccountStatus::from_db("active"),
            Some(AccountStatus::Active)
        );
        assert!(AccountStatus::Active.can_authenticate());
        assert!(!AccountStatus::PendingVerification.can_authenticate());
        assert!(!AccountStatus::Suspended.can_authenticate());
        assert!(!AccountStatus::Deleted.can_authenticate());
        assert_eq!(AccountStatus::from_db("nonsense"), None);
    }

    #[test]
    fn lockout_allows_below_threshold() {
        let p = LockoutPolicy::new(5, Duration::from_secs(300), Duration::from_secs(60));
        assert_eq!(p.evaluate(4, t(100), t(200)), Ok(()));
    }

    #[test]
    fn lockout_triggers_at_threshold() {
        let p = LockoutPolicy::new(5, Duration::from_secs(300), Duration::from_secs(60));
        // 5th failure at t=100; lockout ends at t=160; t=120 is locked.
        assert_eq!(
            p.evaluate(5, t(100), t(120)),
            Err(AuthPolicyError::LockedOut)
        );
        // After the base lockout expires, attempts resume.
        assert_eq!(p.evaluate(5, t(100), t(161)), Ok(()));
    }

    #[test]
    fn lockout_backoff_grows() {
        let p = LockoutPolicy::new(5, Duration::from_secs(300), Duration::from_secs(60));
        // 7 failures -> lockout 240s -> t=100+240=340 unlocks.
        assert_eq!(
            p.evaluate(7, t(100), t(300)),
            Err(AuthPolicyError::LockedOut)
        );
        assert_eq!(p.evaluate(7, t(100), t(341)), Ok(()));
    }

    #[test]
    fn refresh_rotation_and_reuse() {
        let live = "hash-live";
        let old = "hash-old";
        let older = "hash-older";
        let ancestors = [old, older];

        assert_eq!(
            evaluate_refresh(live, live, &ancestors),
            RefreshDecision::Rotate
        );
        assert_eq!(
            evaluate_refresh(old, live, &ancestors),
            RefreshDecision::ReuseDetected
        );
        assert_eq!(
            evaluate_refresh("hash-nobody", live, &ancestors),
            RefreshDecision::Unknown
        );
    }
}
