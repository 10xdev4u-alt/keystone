//! Pure domain types for keystone. No I/O, no HTTP — only types and invariants.
//!
//! The database stores these as constrained TEXT values (CHECK constraints in
//! SQL). The Rust enum is the single source of truth; the SQL CHECK list must
//! stay in sync (enforced by a schema test once query macros land).

use std::fmt;
use std::str::FromStr;

/// Platform roles. One column, one enum, one source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Moderator,
    Admin,
    SuperAdmin,
}

impl Role {
    pub const ALL: [Role; 4] = [Role::User, Role::Moderator, Role::Admin, Role::SuperAdmin];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Moderator => "moderator",
            Role::Admin => "admin",
            Role::SuperAdmin => "super_admin",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = UnknownRole;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Role::User),
            "moderator" => Ok(Role::Moderator),
            "admin" => Ok(Role::Admin),
            "super_admin" => Ok(Role::SuperAdmin),
            _ => Err(UnknownRole(s.to_owned())),
        }
    }
}

/// Error returned when a role string is not a known role.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown role: {0}")]
pub struct UnknownRole(pub String);

/// Lifecycle status of a user account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    PendingVerification,
    Active,
    Suspended,
    Deleted,
}

impl UserStatus {
    pub const ALL: [UserStatus; 4] = [
        UserStatus::PendingVerification,
        UserStatus::Active,
        UserStatus::Suspended,
        UserStatus::Deleted,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            UserStatus::PendingVerification => "pending_verification",
            UserStatus::Active => "active",
            UserStatus::Suspended => "suspended",
            UserStatus::Deleted => "deleted",
        }
    }
}

impl fmt::Display for UserStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for UserStatus {
    type Err = UnknownUserStatus;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending_verification" => Ok(UserStatus::PendingVerification),
            "active" => Ok(UserStatus::Active),
            "suspended" => Ok(UserStatus::Suspended),
            "deleted" => Ok(UserStatus::Deleted),
            _ => Err(UnknownUserStatus(s.to_owned())),
        }
    }
}

/// Error returned when a status string is not a known status.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown user status: {0}")]
pub struct UnknownUserStatus(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_round_trips() {
        for role in Role::ALL {
            assert_eq!(role.to_string().parse::<Role>().unwrap(), role);
        }
        assert_eq!("admin".parse::<Role>().unwrap(), Role::Admin);
        assert!("owner".parse::<Role>().is_err());
    }

    #[test]
    fn status_round_trips() {
        for status in UserStatus::ALL {
            assert_eq!(status.to_string().parse::<UserStatus>().unwrap(), status);
        }
        assert_eq!("active".parse::<UserStatus>().unwrap(), UserStatus::Active);
        assert!("banned".parse::<UserStatus>().is_err());
    }
}
