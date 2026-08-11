//! Repositories — the only place SQL lives.
//!
//! Every query is bound and typed; dynamic SQL is banned. Runtime queries here
//! are deliberate for this milestone; the plan's compile-time `query!` pass
//! (sqlx offline metadata) lands as a dedicated hardening PR so it can be
//! reviewed on its own.

pub mod bookmarks;
pub mod careers;
pub mod comments;
pub mod communities;
pub mod community_posts;
pub mod links;
pub mod moderation;
pub mod organizations;
pub mod polls;
pub mod posts;
pub mod profiles;
pub mod qa;
pub mod reactions;
pub mod reports;
pub mod reviews;
pub mod series;
pub mod sessions;
pub mod tags;
pub mod users;

/// Typed error for all repository operations.
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("email is already registered")]
    EmailTaken,
    #[error("conflicting unique value: {0}")]
    UniqueViolation(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}
