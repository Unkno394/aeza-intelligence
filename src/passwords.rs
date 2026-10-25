//! Password hashes in `public.admins.password_hash` (DEFAULT_AUTH login).
//!
//! Argon2id in the PHC string format, so the parameters and the salt travel
//! with each hash and can change without a migration.

use std::sync::OnceLock;

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

pub const MIN_LENGTH: usize = 16;

pub fn hash(password: &str) -> String {
    Argon2::default()
        .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
        .expect("Argon2 with default parameters hashes any password")
        .to_string()
}

/// Whether `password` matches `hash`. Without a stored hash a throwaway one is
/// checked instead, so an unknown login takes as long as a wrong password.
pub fn verify(password: &str, hash: Option<&str>) -> bool {
    static UNUSABLE: OnceLock<String> = OnceLock::new();
    let fallback = || UNUSABLE.get_or_init(|| self::hash("no admin has this password"));
    let parsed = PasswordHash::new(hash.unwrap_or_else(|| fallback()));
    let matches = parsed.is_ok_and(|parsed| {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    });
    matches && hash.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_verifies_only_its_password() {
        let stored = hash("correct horse battery");
        assert!(stored.starts_with("$argon2id$"));
        assert_ne!(stored, hash("correct horse battery"), "every hash has its own salt");
        assert!(verify("correct horse battery", Some(&stored)));
        assert!(!verify("correct horse batterY", Some(&stored)));
        assert!(!verify("anything", None));
        assert!(!verify("anything", Some("not a hash")));
    }
}
