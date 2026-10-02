use anyhow::{Result, anyhow, bail};
use axum::http::{HeaderMap, header};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use hmac::Hmac;
use parking_lot::RwLock;
use pbkdf2::pbkdf2;
use rand::random;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{collections::HashMap, sync::Arc};
use subtle::ConstantTimeEq;

const COOKIE_NAME: &str = "bilicraft_web_session";
const HASH_ALGORITHM: &str = "pbkdf2-sha256";
const ITERATIONS: u32 = 210_000;
const SALT_BYTES: usize = 16;
const HASH_BYTES: usize = 32;
const SESSION_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebAuthSecrets {
    #[serde(default)]
    pub password_hash: Option<String>,
}

#[derive(Clone, Default)]
pub struct WebAuthState {
    sessions: Arc<RwLock<HashMap<String, i64>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthState {
    SetupRequired,
    LoginRequired,
    Authenticated,
}

impl WebAuthState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self, secrets: &WebAuthSecrets, headers: &HeaderMap) -> AuthState {
        if secrets.password_hash.is_none() {
            return AuthState::SetupRequired;
        }
        if self.session_from_headers(headers).is_some() {
            AuthState::Authenticated
        } else {
            AuthState::LoginRequired
        }
    }

    pub fn set_password(
        &self,
        secrets: &mut WebAuthSecrets,
        password: &str,
        confirmation: &str,
        allow_replace: bool,
    ) -> Result<()> {
        validate_password(password, confirmation)?;
        if secrets.password_hash.is_some() && !allow_replace {
            bail!("web password is already configured");
        }
        secrets.password_hash = Some(hash_password(password)?);
        Ok(())
    }

    pub fn verify_password(&self, secrets: &WebAuthSecrets, password: &str) -> bool {
        secrets
            .password_hash
            .as_deref()
            .and_then(|encoded| verify_encoded(encoded, password).ok())
            .unwrap_or(false)
    }

    pub fn create_session(&self) -> String {
        let bytes: [u8; 32] = random();
        let token = URL_SAFE_NO_PAD.encode(bytes);
        self.sessions
            .write()
            .insert(token.clone(), Utc::now().timestamp() + SESSION_TTL_SECONDS);
        token
    }

    pub fn clear_session(&self, headers: &HeaderMap) {
        if let Some(token) = self.session_from_headers(headers) {
            self.sessions.write().remove(&token);
        }
    }

    pub fn session_cookie(token: &str) -> String {
        format!(
            "{COOKIE_NAME}={token}; Path=/; Max-Age={SESSION_TTL_SECONDS}; HttpOnly; SameSite=Strict"
        )
    }

    pub fn clear_cookie() -> &'static str {
        "bilicraft_web_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict"
    }

    pub fn session_from_headers(&self, headers: &HeaderMap) -> Option<String> {
        let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
        let token = cookie.split(';').find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            (name == COOKIE_NAME).then_some(value.to_owned())
        })?;
        let now = Utc::now().timestamp();
        let mut sessions = self.sessions.write();
        sessions.retain(|_, expires_at| *expires_at > now);
        sessions.contains_key(&token).then_some(token)
    }
}

fn validate_password(password: &str, confirmation: &str) -> Result<()> {
    let length = password.chars().count();
    if !(8..=256).contains(&length) {
        bail!("web password must contain 8 to 256 characters");
    }
    if password != confirmation {
        bail!("web password confirmation does not match");
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String> {
    let salt: [u8; SALT_BYTES] = random();
    let mut digest = [0u8; HASH_BYTES];
    pbkdf2::<HmacSha256>(password.as_bytes(), &salt, ITERATIONS, &mut digest)
        .map_err(|_| anyhow!("failed to hash web password"))?;
    Ok(format!(
        "{HASH_ALGORITHM}${ITERATIONS}${}${}",
        URL_SAFE_NO_PAD.encode(salt),
        URL_SAFE_NO_PAD.encode(digest)
    ))
}

fn verify_encoded(encoded: &str, password: &str) -> Result<bool> {
    let mut parts = encoded.split('$');
    if parts.next() != Some(HASH_ALGORITHM) {
        bail!("unsupported web password hash");
    }
    let iterations = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value >= 100_000)
        .ok_or_else(|| anyhow!("invalid web password hash iterations"))?;
    let salt = URL_SAFE_NO_PAD.decode(parts.next().unwrap_or_default())?;
    let expected = URL_SAFE_NO_PAD.decode(parts.next().unwrap_or_default())?;
    if parts.next().is_some() || salt.len() != SALT_BYTES || expected.len() != HASH_BYTES {
        bail!("invalid web password hash");
    }
    let mut digest = [0u8; HASH_BYTES];
    pbkdf2::<HmacSha256>(password.as_bytes(), &salt, iterations, &mut digest)
        .map_err(|_| anyhow!("failed to verify web password"))?;
    Ok(digest.ct_eq(&expected).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_verifies_password_without_storing_plaintext() {
        let auth = WebAuthState::new();
        let mut secrets = WebAuthSecrets::default();
        auth.set_password(&mut secrets, "correct horse", "correct horse", false)
            .unwrap();
        let encoded = secrets.password_hash.as_ref().unwrap();
        assert!(!encoded.contains("correct horse"));
        assert!(auth.verify_password(&secrets, "correct horse"));
        assert!(!auth.verify_password(&secrets, "wrong password"));
    }

    #[test]
    fn rejects_weak_or_mismatched_passwords() {
        let auth = WebAuthState::new();
        let mut secrets = WebAuthSecrets::default();
        assert!(
            auth.set_password(&mut secrets, "short", "short", false)
                .is_err()
        );
        assert!(
            auth.set_password(&mut secrets, "long enough", "different", false)
                .is_err()
        );
    }
}
