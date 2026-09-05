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
