//! Identity & auth primitives for Keystone.
//!
//! - `password`: Argon2id hashing/verification (PHC strings, tuned params).
//! - `jwt`: HS256-signed access tokens with pinned issuer/audience, keys
//!   loaded once at boot.
//! - `tokens`: opaque refresh tokens (random, hashed before storage).
//! - `service`: pure auth policy — lockout, session rotation, reuse detection.
//! - `email`: input validation for addresses.
//!
//! Session persistence, endpoints and OAuth live in later milestones; this
//! crate only holds the cryptographic core and policy rules so they can be
//! reviewed in isolation.

#![forbid(unsafe_code)]

pub mod email;
pub mod jwt;
pub mod password;
pub mod service;
pub mod tokens;
