//! Access tokens — HS256-signed JWT.
//!
//! v1 uses HS256 (HMAC-SHA256) on the stable `jwt-compact` 0.8 line. This is a
//! deliberate, documented choice:
//!
//! - The only party that signs AND verifies access tokens in v1 is the API
//!   itself (no gateway, no separate auth service), so a symmetric key is
//!   cryptographically sound.
//! - It avoids the `rsa` crate entirely (RUSTSEC-2023-0071, no fix available):
//!   jwt-compact 0.8 only ships RSA/ES256K algorithm features, and depending
//!   on a beta release for ES256/EdDSA is not acceptable for this path.
//! - Algorithm pinning is enforced by the library: the `alg` header is filled
//!   automatically and compared at verification, so algorithm-switching
//!   attacks are structurally impossible.
//!
//! Upgrade path (when a second verifier exists): switch `Hs256`/`Hs256Key` for
//! an asymmetric algorithm behind this same `AccessTokenService` boundary and
//! rotate the key. No call-site changes.

use jwt_compact::alg::{Hs256, Hs256Key};
use jwt_compact::{AlgorithmExt, Claims, Header, TimeOptions, UntrustedToken};
use keystone_config::JwtConfig;
use zeroize::Zeroizing;

const MIN_SECRET_LEN: usize = 32; // 256 bits — HMAC keys below this are rejected.

/// Claims carried in an access token. `sub`/`iss`/`aud` are registered claim
/// names carried in the custom payload (this crate version has no dedicated
/// struct for them); `role`/`imp` are Keystone-specific.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct AccessClaims {
    #[serde(rename = "sub")]
    subject: String,
    #[serde(rename = "iss", default, skip_serializing_if = "Option::is_none")]
    issuer: Option<String>,
    #[serde(rename = "aud", default, skip_serializing_if = "Option::is_none")]
    audience: Option<Vec<String>>,
    role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    imp: Option<String>,
}

/// A verified access token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessToken {
    pub user_id: String,
    pub role: String,
    pub impersonator_id: Option<String>,
}

/// The loaded HMAC secret. Held once at boot, never re-read per request.
#[derive(Debug, Clone)]
pub struct JwtKeys {
    secret: Hs256Key,
}

impl JwtKeys {
    /// Load from config: base64-of-bytes wins, then a file containing base64.
    pub fn from_config(config: &JwtConfig) -> Result<Self, JwtError> {
        let b64 = match (&config.private_key_b64, &config.private_key_path) {
            (Some(b64), _) => b64.clone(),
            (None, Some(path)) => std::fs::read_to_string(path)
                .map_err(|e| JwtError::Key(format!("cannot read key file {path}: {e}")))?,
            (None, None) => {
                return Err(JwtError::Key(
                    "no JWT key configured (set JWT_PRIVATE_KEY_B64 or JWT_PRIVATE_KEY_PATH)"
                        .into(),
                ));
            }
        };
        Self::from_base64(b64.trim())
    }

    /// Load from a base64-encoded secret (dev/tests).
    pub fn from_base64(b64: &str) -> Result<Self, JwtError> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| JwtError::Key(format!("JWT key is not valid base64: {e}")))?;
        Self::from_secret(&bytes)
    }

    /// Load from raw secret bytes; rejects keys weaker than 256 bits.
    pub fn from_secret(bytes: &[u8]) -> Result<Self, JwtError> {
        if bytes.len() < MIN_SECRET_LEN {
            return Err(JwtError::Key(format!(
                "JWT secret must be at least {MIN_SECRET_LEN} bytes, got {}",
                bytes.len()
            )));
        }
        let zeroized = Zeroizing::new(bytes.to_vec());
        Ok(Self {
            secret: Hs256Key::new(&zeroized[..]),
        })
    }
}

/// Signs and verifies access tokens with pinned issuer/audience/TTL.
#[derive(Debug, Clone)]
pub struct AccessTokenService {
    alg: Hs256,
    secret: Hs256Key,
    issuer: String,
    audience: String,
    ttl: chrono::TimeDelta,
    time_options: TimeOptions,
}

impl AccessTokenService {
    pub fn new(config: &JwtConfig, keys: JwtKeys) -> Self {
        Self {
            alg: Hs256,
            secret: keys.secret,
            issuer: config.issuer.clone(),
            audience: config.audience.clone(),
            ttl: chrono::TimeDelta::seconds(config.access_expiration_secs.max(1)),
            time_options: TimeOptions::default(),
        }
    }

    /// Issue an access token for a user.
    pub fn issue(
        &self,
        user_id: &str,
        role: &str,
        impersonator_id: Option<&str>,
    ) -> Result<String, JwtError> {
        let claims = Claims::new(AccessClaims {
            subject: user_id.to_owned(),
            issuer: Some(self.issuer.clone()),
            audience: Some(vec![self.audience.clone()]),
            role: role.to_owned(),
            imp: impersonator_id.map(str::to_owned),
        })
        .set_duration_and_issuance(&self.time_options, self.ttl);

        self.alg
            .token(&Header::empty(), &claims, &self.secret)
            .map_err(|e| JwtError::Creation(e.to_string()))
    }

    /// Verify a token: signature, expiry, issuer, audience, subject.
    pub fn verify(&self, token_str: &str) -> Result<AccessToken, JwtError> {
        let untrusted =
            UntrustedToken::new(token_str).map_err(|e| JwtError::Invalid(e.to_string()))?;
        let token: jwt_compact::Token<AccessClaims> = self
            .alg
            .validator(&self.secret)
            .validate(&untrusted)
            .map_err(|e| JwtError::Invalid(e.to_string()))?;

        let claims = token.claims();
        claims
            .validate_expiration(&self.time_options)
            .map_err(|e| JwtError::Invalid(e.to_string()))?;
        if claims.not_before.is_some() {
            claims
                .validate_maturity(&self.time_options)
                .map_err(|e| JwtError::Invalid(e.to_string()))?;
        }

        let custom = &claims.custom;
        if custom.issuer.as_deref() != Some(self.issuer.as_str()) {
            return Err(JwtError::Invalid("issuer mismatch".into()));
        }
        let audience_ok = custom
            .audience
            .as_deref()
            .is_some_and(|auds| auds.iter().any(|a| a == &self.audience));
        if !audience_ok {
            return Err(JwtError::Invalid("audience mismatch".into()));
        }

        Ok(AccessToken {
            user_id: custom.subject.clone(),
            role: custom.role.clone(),
            impersonator_id: custom.imp.clone(),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JwtError {
    #[error("key error: {0}")]
    Key(String),
    #[error("token creation failed: {0}")]
    Creation(String),
    #[error("invalid token: {0}")]
    Invalid(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use keystone_config::JwtConfig;

    const SECRET_B64: &str = "c2VjcmV0LXNlY3JldC1zZWNyZXQtc2VjcmV0LXNlY3JldC0xMjM0NTY3ODkw";
    // ^ 48-byte secret, tests only — never used in production.

    fn config() -> JwtConfig {
        JwtConfig {
            issuer: "keystone-test".into(),
            audience: "keystone-api".into(),
            access_expiration_secs: 900,
            refresh_expiration_secs: 604_800,
            private_key_b64: Some(SECRET_B64.into()),
            private_key_path: None,
        }
    }

    fn service() -> AccessTokenService {
        AccessTokenService::new(&config(), JwtKeys::from_config(&config()).unwrap())
    }

    #[test]
    fn issue_and_verify_round_trip() {
        let svc = service();
        let token = svc
            .issue("user-123", "admin", Some("imp-9"))
            .expect("issue must work");
        let parsed = svc.verify(&token).expect("verify must work");
        assert_eq!(parsed.user_id, "user-123");
        assert_eq!(parsed.role, "admin");
        assert_eq!(parsed.impersonator_id.as_deref(), Some("imp-9"));
    }

    #[test]
    fn tampered_token_is_rejected() {
        let svc = service();
        let token = svc.issue("user-123", "user", None).unwrap();
        let mut chars: Vec<char> = token.chars().collect();
        let last = chars.len() - 1;
        chars[last] = if chars[last] == 'a' { 'b' } else { 'a' };
        let tampered: String = chars.into_iter().collect();
        assert!(svc.verify(&tampered).is_err());
    }

    #[test]
    fn wrong_audience_is_rejected() {
        let svc = service();
        let token = svc.issue("u1", "user", None).unwrap();
        let mut other_config = config();
        other_config.audience = "someone-else".into();
        let other =
            AccessTokenService::new(&other_config, JwtKeys::from_config(&other_config).unwrap());
        assert!(other.verify(&token).is_err());
    }

    #[test]
    fn weak_secret_is_rejected() {
        assert!(JwtKeys::from_secret(b"short").is_err());
    }

    #[test]
    fn missing_key_config_errors() {
        let mut cfg = config();
        cfg.private_key_b64 = None;
        cfg.private_key_path = None;
        let err = JwtKeys::from_config(&cfg).unwrap_err();
        assert!(err.to_string().contains("JWT"));
    }

    #[test]
    fn same_secret_verifies_across_services() {
        let svc1 = service();
        let cfg2 = config();
        let svc2 = AccessTokenService::new(&cfg2, JwtKeys::from_config(&cfg2).unwrap());
        let token = svc1.issue("u1", "moderator", None).unwrap();
        let parsed = svc2.verify(&token).unwrap();
        assert_eq!(parsed.role, "moderator");
    }
}
