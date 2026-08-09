//! Email validation.
//!
//! Deliberately simple and dependency-free: no regex bomb, no RFC 5322
//! rabbit hole. We check shape (local@domain with a dot in the domain) and
//! length; deliverability is the mailer's job, uniqueness is the DB's.

const MAX_EMAIL_LEN: usize = 254; // RFC 5321 limit, still the sane cap.

/// Validate an email address shape. Returns an error message on failure.
pub fn validate(email: &str) -> Result<(), &'static str> {
    let trimmed = email.trim();
    if trimmed.is_empty() {
        return Err("email is required");
    }
    if trimmed.len() > MAX_EMAIL_LEN {
        return Err("email is too long");
    }
    let Some(at) = trimmed.rfind('@') else {
        return Err("email must contain '@'");
    };
    let local = &trimmed[..at];
    let domain = &trimmed[at + 1..];
    if local.is_empty() {
        return Err("email is missing a local part");
    }
    if domain.is_empty() || !domain.contains('.') {
        return Err("email is missing a valid domain");
    }
    // No whitespace or control characters anywhere.
    if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("email must not contain whitespace");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plausible_addresses() {
        assert!(validate("ada@example.com").is_ok());
        assert!(validate("a.b+c@sub.example.co.uk").is_ok());
        assert!(validate(" x@example.com ").is_ok()); // trimmed
    }

    #[test]
    fn rejects_malformed_addresses() {
        assert!(validate("").is_err());
        assert!(validate("nope").is_err());
        assert!(validate("@example.com").is_err());
        assert!(validate("a@b").is_err());
        assert!(validate("a b@example.com").is_err());
        assert!(validate(&format!("{}@example.com", "x".repeat(MAX_EMAIL_LEN))).is_err());
    }
}
