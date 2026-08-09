//! Password hashing — Argon2id.
//!
//! Uses the standard PHC string format (e.g. `$argon2id$v=19$m=65536,t=3,p=4$...`)
//! so hashes are self-describing and portable. Verification is constant-time
//! with respect to the password via the underlying implementation.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher as _, PasswordVerifier as _, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use keystone_config::Argon2Config;

const MIN_PASSWORD_LEN: usize = 8;
const MAX_PASSWORD_LEN: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("password must be between {MIN_PASSWORD_LEN} and {MAX_PASSWORD_LEN} characters")]
    Policy,
    #[error("failed to hash password: {0}")]
    Hash(String),
    #[error("malformed password hash: {0}")]
    MalformedHash(String),
}

/// Validates a candidate password against the policy.
pub fn validate(password: &str) -> Result<(), PasswordError> {
    let len = password.chars().count();
    if !(MIN_PASSWORD_LEN..=MAX_PASSWORD_LEN).contains(&len) {
        return Err(PasswordError::Policy);
    }
    Ok(())
}

/// Argon2id hasher with explicitly tuned parameters.
#[derive(Debug, Clone)]
pub struct PasswordHasher {
    params: Params,
}

impl PasswordHasher {
    pub fn from_config(config: &Argon2Config) -> Result<Self, PasswordError> {
        let params = Params::new(
            config.memory_kib,
            config.iterations,
            config.parallelism,
            Some(32),
        )
        .map_err(|e| PasswordError::Hash(e.to_string()))?;
        Ok(Self { params })
    }

    /// Hash a password into a PHC string. The salt is fresh random per call.
    pub fn hash(&self, password: &str) -> Result<String, PasswordError> {
        validate(password)?;
        let argon2 = self.argon2();
        let salt = SaltString::generate(&mut OsRng);
        let hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| PasswordError::Hash(e.to_string()))?;
        Ok(hash.to_string())
    }

    /// Verify a password against a stored PHC string. Returns Ok(false) for
    /// malformed or unknown-parameter hashes rather than panicking.
    pub fn verify(&self, password: &str, stored: &str) -> Result<bool, PasswordError> {
        let parsed =
            PasswordHash::new(stored).map_err(|e| PasswordError::MalformedHash(e.to_string()))?;
        Ok(self
            .argon2()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    }

    fn argon2(&self) -> Argon2<'static> {
        Argon2::new(Algorithm::Argon2id, Version::V0x13, self.params.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keystone_config::Argon2Config;

    fn hasher() -> PasswordHasher {
        PasswordHasher::from_config(&Argon2Config {
            memory_kib: 19_456,
            iterations: 2,
            parallelism: 1,
        })
        .expect("params must be valid")
    }

    #[test]
    fn hash_and_verify_round_trip() {
        let h = hasher();
        let hash = h.hash("correct horse battery staple").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(h.verify("correct horse battery staple", &hash).unwrap());
        assert!(!h.verify("wrong password", &hash).unwrap());
    }

    #[test]
    fn hashes_are_unique_per_call() {
        let h = hasher();
        let a = h.hash("same password").unwrap();
        let b = h.hash("same password").unwrap();
        assert_ne!(a, b, "random salt must produce distinct hashes");
        assert!(h.verify("same password", &a).unwrap());
        assert!(h.verify("same password", &b).unwrap());
    }

    #[test]
    fn policy_rejects_short_and_huge() {
        assert!(matches!(validate("short"), Err(PasswordError::Policy)));
        let huge = "x".repeat(MAX_PASSWORD_LEN + 1);
        assert!(matches!(validate(&huge), Err(PasswordError::Policy)));
        assert!(validate(&"a".repeat(MIN_PASSWORD_LEN)).is_ok());
    }

    #[test]
    fn malformed_hash_returns_error_not_panic() {
        let h = hasher();
        assert!(matches!(
            h.verify("whatever", "not-a-phc-string"),
            Err(PasswordError::MalformedHash(_))
        ));
    }
}
