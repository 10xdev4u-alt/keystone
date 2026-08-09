//! Identity & auth primitives for Keystone.
//!
//! - `password`: Argon2id hashing/verification (PHC strings, tuned params).
//! - `jwt`: ES256-signed access tokens with pinned issuer/audience, keys
//!   loaded once at boot.
//!
//! Session persistence, endpoints and OAuth live in later milestones; this
//! crate only holds the cryptographic core so it can be reviewed in isolation.

#![forbid(unsafe_code)]

pub mod jwt;
pub mod password;
