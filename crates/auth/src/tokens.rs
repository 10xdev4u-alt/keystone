//! Opaque refresh tokens.
//!
//! A refresh token is 32 random bytes, base64url-encoded for transport. The
//! database stores only its SHA-256 hash — a breach of the sessions table
//! yields hashes that cannot be replayed, and the raw token is never logged.
//!
//! The token itself is the credential: it is delivered to the client exactly
//! once at issuance (login / rotation) and never stored server-side in clear.

use zeroize::Zeroizing;

pub const REFRESH_TOKEN_BYTES: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("failed to gather randomness: {0}")]
    Generation(String),
}

/// Generate a fresh opaque refresh token (base64url, no padding).
pub fn generate_refresh_token() -> Result<String, TokenError> {
    let mut bytes = Zeroizing::new([0u8; REFRESH_TOKEN_BYTES]);
    getrandom::fill(&mut bytes[..]).map_err(|e| TokenError::Generation(e.to_string()))?;
    use base64::Engine;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes.as_ref()))
}

/// SHA-256 hash of a refresh token, hex-encoded — what gets stored.
pub fn hash_refresh_token(token: &str) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(token.as_bytes());
    hex::encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_and_well_formed() {
        let a = generate_refresh_token().unwrap();
        let b = generate_refresh_token().unwrap();
        assert_ne!(a, b);
        // 32 bytes -> 43 base64url chars with no padding.
        assert_eq!(a.len(), 43);
        assert!(!a.contains('+') && !a.contains('/') && !a.contains('='));
    }

    #[test]
    fn hash_is_hex_and_deterministic() {
        let t = generate_refresh_token().unwrap();
        let h1 = hash_refresh_token(&t);
        let h2 = hash_refresh_token(&t);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_does_not_contain_plaintext() {
        let t = generate_refresh_token().unwrap();
        let h = hash_refresh_token(&t);
        assert_ne!(h, t);
        // A different token hashes differently (no collisions in practice).
        let other = generate_refresh_token().unwrap();
        assert_ne!(hash_refresh_token(&other), h);
    }
}
