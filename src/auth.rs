use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::{config::Config, error::AppError, state::AppState};

pub const COOKIE_NAME: &str = "diary_session";

type HmacSha256 = Hmac<Sha256>;

fn sign(secret: &[u8], payload: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts any key length");
    mac.update(payload.as_bytes());
    B64.encode(mac.finalize().into_bytes())
}

/// A session token is `base64(expiry_unix_seconds).base64(hmac)`. There is no
/// server-side session store: rotating `DIARY_SECRET` invalidates every session.
pub fn issue_token(config: &Config) -> String {
    let expires = crate::now() + config.session_days * 86_400;
    let payload = B64.encode(expires.to_string());
    let signature = sign(&config.secret, &payload);
    format!("{payload}.{signature}")
}

pub fn verify_token(config: &Config, token: &str) -> bool {
    let Some((payload, signature)) = token.split_once('.') else {
        return false;
    };
    let expected = sign(&config.secret, payload);
    if !bool::from(expected.as_bytes().ct_eq(signature.as_bytes())) {
        return false;
    }
    let Ok(decoded) = B64.decode(payload) else {
        return false;
    };
    let Ok(expires) = String::from_utf8_lossy(&decoded).parse::<i64>() else {
        return false;
    };
    expires > crate::now()
}

pub fn credentials_match(config: &Config, username: &str, password: &str) -> bool {
    let user_ok = username.as_bytes().ct_eq(config.username.as_bytes());
    let pass_ok = password.as_bytes().ct_eq(config.password.as_bytes());
    bool::from(user_ok & pass_ok)
}

pub fn session_cookie(config: &Config, token: &str) -> String {
    let max_age = config.session_days * 86_400;
    let secure = if config.secure_cookie { "; Secure" } else { "" };
    format!(
        "{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    )
}

pub fn clear_cookie(config: &Config) -> String {
    let secure = if config.secure_cookie { "; Secure" } else { "" };
    format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}")
}

fn cookie_value<'a>(parts: &'a Parts, name: &str) -> Option<&'a str> {
    parts
        .headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| k.trim() == name)
        .map(|(_, v)| v.trim())
}

/// Extractor that rejects the request unless a valid session cookie is present.
pub struct Session;

impl FromRequestParts<AppState> for Session {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = cookie_value(parts, COOKIE_NAME).ok_or(AppError::Unauthorized)?;
        if verify_token(&state.config, token) {
            Ok(Session)
        } else {
            Err(AppError::Unauthorized)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{credentials_match, issue_token, verify_token};
    use crate::config::{BackupConfig, Config};

    fn config(session_days: i64) -> Config {
        Config {
            username: "narl".into(),
            password: "hunter2".into(),
            secret: b"a-secret-of-at-least-16-bytes".to_vec(),
            bind: "127.0.0.1:0".parse().unwrap(),
            data_dir: ".".into(),
            session_days,
            max_upload_bytes: 1,
            secure_cookie: false,
            backup: BackupConfig {
                device_name: "narl-diary".into(),
                interval: None,
                debounce: std::time::Duration::from_secs(0),
                prune: false,
            },
        }
    }

    #[test]
    fn a_fresh_token_verifies() {
        let config = config(30);
        assert!(verify_token(&config, &issue_token(&config)));
    }

    #[test]
    fn an_expired_token_does_not() {
        let config = config(-1);
        assert!(!verify_token(&config, &issue_token(&config)));
    }

    #[test]
    fn rotating_the_secret_invalidates_every_session() {
        let token = issue_token(&config(30));
        let mut rotated = config(30);
        rotated.secret = b"a-different-secret-entirely".to_vec();
        assert!(!verify_token(&rotated, &token));
    }

    #[test]
    fn a_tampered_token_does_not_verify() {
        let config = config(30);
        let token = issue_token(&config);
        let (payload, signature) = token.split_once('.').unwrap();

        // A far-future expiry the holder signed themselves.
        let forged = base64::Engine::encode(&super::B64, "99999999999");
        assert!(!verify_token(&config, &format!("{forged}.{signature}")));

        // The real expiry with the last byte of the signature flipped.
        let mut bytes = signature.as_bytes().to_vec();
        bytes[0] = if bytes[0] == b'A' { b'B' } else { b'A' };
        let flipped = String::from_utf8(bytes).unwrap();
        assert!(!verify_token(&config, &format!("{payload}.{flipped}")));
    }

    #[test]
    fn malformed_tokens_are_rejected_rather_than_panicking() {
        let config = config(30);
        for token in ["", ".", "no-dot", "!!!.!!!", "a.b.c", &".".repeat(100)] {
            assert!(!verify_token(&config, token), "{token:?}");
        }
    }

    #[test]
    fn credentials_must_match_exactly() {
        let config = config(30);
        assert!(credentials_match(&config, "narl", "hunter2"));
        assert!(!credentials_match(&config, "narl", "hunter"));
        assert!(!credentials_match(&config, "narl", "hunter22"));
        assert!(!credentials_match(&config, "nar", "hunter2"));
        assert!(!credentials_match(&config, "", ""));
    }
}
