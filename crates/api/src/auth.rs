//! Server-side operator authentication, session management, and verified principal extraction.
//!
//! Replaces unverified client-asserted `X-Person-Id` headers with server-bound operator identity:
//! 1. Server-issued opaque session tokens (`POST /auth/login`, `POST /auth/logout`, `GET /auth/me`),
//!    sent via `HttpOnly; SameSite=Lax` cookie or `Authorization: Bearer <session_token>`.
//! 2. Optional per-operator static tokens via `API_USER_TOKENS="person1:tok1,person2:tok2"`.
//! 3. Optional trusted authenticating reverse-proxy header via `TRUSTED_PROXY_AUTH_HEADER`
//!    (never `x-person-id`).
//! 4. Shared `API_TOKEN` alone no longer authorizes operator impersonation via `X-Person-Id`
//!    unless `ALLOW_UNVERIFIED_PERSON_HEADER=true` is explicitly set for legacy dev tooling.

use std::{
    collections::HashMap,
    num::NonZeroU32,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};

use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use common::db::sqlx::{self, types::Uuid, PgPool};
use ring::pbkdf2;
use serde::Deserialize;
use serde_json::json;

use crate::{
    error::{db_err, err, ApiResult},
    AppState,
};

pub const SESSION_COOKIE_NAME: &str = "alerthub_session";
const SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const MAX_SESSIONS: usize = 4_096;
const PBKDF2_ITERATIONS: u32 = 100_000;
const DEFAULT_USER_PASSWORD: &str = "haslo";

static AUTH_USERS_SEEDED: AtomicBool = AtomicBool::new(false);

/// 20 domyślnych kont operatorów (`(username / person_id, display_name)`).
/// Wszystkie konta są przy starcie zapisywane w tabeli `auth_users` w Postgresie
/// jako zahashowane hasło (`PBKDF2-HMAC-SHA256`, 100 000 iteracji + unikalna losowa sól).
pub const SEED_USERS: [(&str, &str); 20] = [
    ("anna", "Anna Kowalska"),
    ("jan", "Jan Nowak"),
    ("ola", "Ola Wiśniewska"),
    ("piotr", "Piotr Wójcik"),
    ("tomek", "Tomek Kamiński"),
    ("kasia", "Kasia Lewandowska"),
    ("marek", "Marek Zieliński"),
    ("magda", "Magda Szymańska"),
    ("michal", "Michał Woźniak"),
    ("monika", "Monika Dąbrowska"),
    ("pawel", "Paweł Kozłowski"),
    ("agnieszka", "Agnieszka Jankowska"),
    ("krzysztof", "Krzysztof Mazur"),
    ("ewa", "Ewa Kwiatkowska"),
    ("bartek", "Bartek Krawczyk"),
    ("karolina", "Karolina Piotrowska"),
    ("lukasz", "Łukasz Grabowski"),
    ("natalia", "Natalia Nowakowska"),
    ("dawid", "Dawid Pawłowski"),
    ("zuzanna", "Zuzanna Michalska"),
];

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn from_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for i in (0..hex.len()).step_by(2) {
        let byte = u8::from_str_radix(&hex[i..i + 2], 16).ok()?;
        out.push(byte);
    }
    Some(out)
}

pub fn hash_password(password: &str, salt_hex: &str) -> String {
    let salt_bytes = from_hex(salt_hex).unwrap_or_else(|| salt_hex.as_bytes().to_vec());
    let mut out = [0u8; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(PBKDF2_ITERATIONS).expect("non-zero iterations"),
        &salt_bytes,
        password.as_bytes(),
        &mut out,
    );
    to_hex(&out)
}

pub fn verify_password(password: &str, salt_hex: &str, expected_hash_hex: &str) -> bool {
    let Some(salt_bytes) = from_hex(salt_hex) else {
        return false;
    };
    let Some(expected_bytes) = from_hex(expected_hash_hex) else {
        return false;
    };
    pbkdf2::verify(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(PBKDF2_ITERATIONS).expect("non-zero iterations"),
        &salt_bytes,
        password.as_bytes(),
        &expected_bytes,
    )
    .is_ok()
}

pub async fn ensure_auth_users_seeded(db: &PgPool, config_dir: &str) {
    if AUTH_USERS_SEEDED.swap(true, Ordering::SeqCst) {
        return;
    }
    crate::routes::people::ensure_people_seeded(db, config_dir).await;

    for (username, display_name) in SEED_USERS {
        let exists: Option<i32> =
            sqlx::query_scalar("SELECT 1 FROM auth_users WHERE username = $1")
                .bind(username)
                .fetch_optional(db)
                .await
                .ok()
                .flatten();
        if exists.is_some() {
            continue;
        }
        let salt_hex = Uuid::new_v4().simple().to_string();
        let hash_hex = hash_password(DEFAULT_USER_PASSWORD, &salt_hex);
        let _ = sqlx::query(
            "INSERT INTO auth_users (username, person_id, display_name, password_salt, password_hash, created_at) \
             VALUES ($1, $2, $3, $4, $5, now()) ON CONFLICT (username) DO NOTHING",
        )
        .bind(username)
        .bind(username)
        .bind(display_name)
        .bind(&salt_hex)
        .bind(&hash_hex)
        .execute(db)
        .await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    Session,
    UserToken,
    TrustedProxy,
    ServiceToken,
}

/// Authenticated caller attached to every request passing `require_token`.
#[derive(Debug, Clone)]
pub struct AuthenticatedOperator {
    /// Verified operator ID bound by session, per-user token, or trusted proxy header.
    /// `None` when authenticated only with the shared service `API_TOKEN`.
    pub person_id: Option<String>,
    pub method: AuthMethod,
    pub session_token: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionEntry {
    person_id: String,
    expires_at: Instant,
}

#[derive(Clone)]
pub struct AuthStore {
    sessions: Arc<RwLock<HashMap<String, SessionEntry>>>,
    /// Map of `person_id -> token` loaded from `API_USER_TOKENS`.
    user_tokens: Arc<HashMap<String, String>>,
    /// Optional reverse-proxy identity header (lowercased, never `x-person-id`).
    trusted_proxy_header: Option<String>,
    /// Legacy opt-in (`ALLOW_UNVERIFIED_PERSON_HEADER=true`) for dev compatibility.
    allow_unverified_person_header: bool,
}

impl AuthStore {
    pub fn from_env() -> Self {
        let user_tokens = parse_user_tokens(&std::env::var("API_USER_TOKENS").unwrap_or_default());
        let trusted_proxy_header = std::env::var("TRUSTED_PROXY_AUTH_HEADER")
            .ok()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty() && s != "x-person-id");
        let allow_unverified_person_header = matches!(
            std::env::var("ALLOW_UNVERIFIED_PERSON_HEADER")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes"
        );
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            user_tokens: Arc::new(user_tokens),
            trusted_proxy_header,
            allow_unverified_person_header,
        }
    }

    pub fn create_session(&self, person_id: String) -> String {
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let now = Instant::now();
        let entry = SessionEntry {
            person_id,
            expires_at: now + SESSION_TTL,
        };
        let mut map = self.sessions.write().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, v| v.expires_at > now);
        if map.len() >= MAX_SESSIONS {
            if let Some(oldest_key) = map
                .iter()
                .min_by_key(|(_, v)| v.expires_at)
                .map(|(k, _)| k.clone())
            {
                map.remove(&oldest_key);
            }
        }
        map.insert(token.clone(), entry);
        token
    }

    pub fn lookup_session(&self, token: &str) -> Option<String> {
        if token.is_empty() {
            return None;
        }
        let now = Instant::now();
        let map = self.sessions.read().unwrap_or_else(|e| e.into_inner());
        map.get(token)
            .filter(|e| e.expires_at > now)
            .map(|e| e.person_id.clone())
    }

    pub fn revoke_session(&self, token: &str) {
        let mut map = self.sessions.write().unwrap_or_else(|e| e.into_inner());
        map.remove(token);
    }

    pub fn lookup_user_token(&self, presented: &str) -> Option<String> {
        if presented.is_empty() || self.user_tokens.is_empty() {
            return None;
        }
        let mut matched: Option<String> = None;
        for (person_id, expected) in self.user_tokens.iter() {
            if constant_time_eq(presented.as_bytes(), expected.as_bytes()) {
                matched = Some(person_id.clone());
            }
        }
        matched
    }

    pub fn has_user_tokens(&self) -> bool {
        !self.user_tokens.is_empty()
    }

    pub fn verify_user_credential(&self, person_id: &str, token: &str) -> bool {
        let Some(expected) = self.user_tokens.get(person_id) else {
            return false;
        };
        !expected.is_empty() && constant_time_eq(token.as_bytes(), expected.as_bytes())
    }

    pub fn trusted_proxy_header(&self) -> Option<&str> {
        self.trusted_proxy_header.as_deref()
    }

    pub fn allow_unverified_person_header(&self) -> bool {
        self.allow_unverified_person_header
    }
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    std::hint::black_box(diff) == 0
}

fn parse_user_tokens(raw: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in raw.split(',') {
        let pair = pair.trim();
        if let Some((person, tok)) = pair.split_once(':') {
            let person = person.trim();
            let tok = tok.trim();
            if !person.is_empty() && !tok.is_empty() {
                out.insert(person.to_string(), tok.to_string());
            }
        }
    }
    out
}

pub fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            if k.trim() == name && !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// Resolves and verifies the operator `person_id` for mutating actions (`ack`, `resolve`, `step_command`).
///
/// - Uses the authenticated server-side identity (`Session`, `UserToken`, or `TrustedProxy`).
/// - Rejects any mismatched client-supplied `X-Person-Id` header with `403 Forbidden`.
/// - Rejects unverified `X-Person-Id` headers on shared `API_TOKEN` requests with `403 Forbidden`
///   unless `ALLOW_UNVERIFIED_PERSON_HEADER=true` is explicitly enabled.
/// - Verifies that the resolved `person_id` exists in the Postgres `people` table.
pub async fn resolve_verified_person_id(
    st: &AppState,
    auth: &AuthenticatedOperator,
    headers: &HeaderMap,
) -> ApiResult<String> {
    let header_person = headers
        .get("x-person-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let person_id = match &auth.person_id {
        Some(verified) => {
            if let Some(claimed) = header_person {
                if claimed != verified {
                    return Err(err(
                        StatusCode::FORBIDDEN,
                        "operator identity mismatch",
                        "X-Person-Id does not match authenticated operator",
                    ));
                }
            }
            verified.clone()
        }
        None => {
            if !st.auth.allow_unverified_person_header() {
                return Err(err(
                    StatusCode::FORBIDDEN,
                    "unauthenticated operator identity",
                    "authenticate via POST /auth/login or a per-user token; unverified X-Person-Id is not accepted",
                ));
            }
            header_person
                .ok_or_else(|| {
                    err(
                        StatusCode::BAD_REQUEST,
                        "missing X-Person-Id header",
                        "wymagany nagłówek X-Person-Id",
                    )
                })?
                .to_string()
        }
    };

    if person_id.len() > 128 || person_id.chars().any(|c| c.is_control()) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "invalid operator id",
            "nieprawidłowy identyfikator operatora",
        ));
    }

    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM people WHERE id = $1")
        .bind(&person_id)
        .fetch_optional(&st.db)
        .await
        .map_err(db_err)?;
    if exists.is_none() {
        return Err(err(
            StatusCode::FORBIDDEN,
            "unknown operator",
            "operator is not registered in people directory",
        ));
    }

    Ok(person_id)
}

#[derive(Deserialize)]
struct LoginRequest {
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    person_id: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

fn session_cookie_header(value: &str, max_age: u64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE_NAME}={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Public routes (`/auth/login`).
pub fn public_router() -> Router<AppState> {
    Router::new().route("/auth/login", post(login))
}

/// Protected routes (`/auth/me`, `/auth/logout`).
pub fn protected_router() -> Router<AppState> {
    Router::new()
        .route("/auth/me", get(me))
        .route("/auth/logout", post(logout))
}

async fn login(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Response {
    ensure_auth_users_seeded(&st.db, &st.config.config_dir).await;

    let raw_user = req
        .username
        .as_deref()
        .or(req.person_id.as_deref())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    if raw_user.is_empty() || raw_user.len() > 128 || raw_user.chars().any(|c| c.is_control()) {
        return err(
            StatusCode::BAD_REQUEST,
            "invalid username",
            "Podaj poprawny login użytkownika",
        )
        .into_response();
    }

    if let Some(password) = req.password.as_deref() {
        let row: Option<(String, String, String, String)> = match sqlx::query_as(
            "SELECT person_id, display_name, password_salt, password_hash FROM auth_users WHERE LOWER(username) = $1",
        )
        .bind(&raw_user)
        .fetch_optional(&st.db)
        .await
        {
            Ok(r) => r,
            Err(e) => return db_err(e).into_response(),
        };

        let Some((person_id, display_name, salt_hex, hash_hex)) = row else {
            let _ = verify_password(
                password,
                "00000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000000",
            );
            return err(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Nieprawidłowy login lub hasło",
            )
            .into_response();
        };

        if !verify_password(password, &salt_hex, &hash_hex) {
            return err(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Nieprawidłowy login lub hasło",
            )
            .into_response();
        }

        let session_token = st.auth.create_session(person_id.clone());
        let mut resp = (
            StatusCode::OK,
            Json(json!({
                "person_id": person_id,
                "username": raw_user,
                "name": display_name,
                "session_token": session_token,
                "expires_in_secs": SESSION_TTL.as_secs(),
            })),
        )
            .into_response();
        resp.headers_mut().insert(
            header::SET_COOKIE,
            session_cookie_header(&session_token, SESSION_TTL.as_secs()),
        );
        return resp;
    }

    let person_id = &raw_user;
    let presented = req
        .token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
        });

    let Some(presented) = presented else {
        return err(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Podaj hasło logowania",
        )
        .into_response();
    };

    let credential_ok = if st.auth.has_user_tokens() {
        st.auth.verify_user_credential(person_id, presented)
    } else {
        let expected = st.config.api_token.as_str();
        !expected.is_empty() && constant_time_eq(presented.as_bytes(), expected.as_bytes())
    };

    if !credential_ok {
        return err(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Nieprawidłowy login lub hasło",
        )
        .into_response();
    }

    let name: Option<String> = match sqlx::query_scalar("SELECT name FROM people WHERE id = $1")
        .bind(person_id)
        .fetch_optional(&st.db)
        .await
    {
        Ok(n) => n,
        Err(e) => return db_err(e).into_response(),
    };

    let Some(name) = name else {
        return err(
            StatusCode::FORBIDDEN,
            "unknown operator",
            "osoba nie istnieje w katalogu people",
        )
        .into_response();
    };

    let session_token = st.auth.create_session(person_id.to_string());
    let mut resp = (
        StatusCode::OK,
        Json(json!({
            "person_id": person_id,
            "username": person_id,
            "name": name,
            "session_token": session_token,
            "expires_in_secs": SESSION_TTL.as_secs(),
        })),
    )
        .into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie_header(&session_token, SESSION_TTL.as_secs()),
    );
    resp
}

async fn me(Extension(auth): Extension<AuthenticatedOperator>) -> impl IntoResponse {
    Json(json!({
        "person_id": auth.person_id,
        "authenticated": auth.person_id.is_some(),
        "method": format!("{:?}", auth.method),
    }))
}

async fn logout(
    State(st): State<AppState>,
    Extension(auth): Extension<AuthenticatedOperator>,
) -> Response {
    if let Some(tok) = auth.session_token.as_deref() {
        st.auth.revoke_session(tok);
    }
    let mut resp = (StatusCode::OK, Json(json!({ "logged_out": true }))).into_response();
    resp.headers_mut()
        .insert(header::SET_COOKIE, session_cookie_header("", 0));
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store(
        user_tokens: &str,
        proxy_header: Option<&str>,
        allow_unverified: bool,
    ) -> AuthStore {
        AuthStore {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            user_tokens: Arc::new(parse_user_tokens(user_tokens)),
            trusted_proxy_header: proxy_header
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty() && s != "x-person-id"),
            allow_unverified_person_header: allow_unverified,
        }
    }

    #[test]
    fn session_lifecycle_and_revocation() {
        let store = test_store("", None, false);
        let tok = store.create_session("anna".into());
        assert_eq!(tok.len(), 64);
        assert_eq!(store.lookup_session(&tok).as_deref(), Some("anna"));
        store.revoke_session(&tok);
        assert_eq!(store.lookup_session(&tok), None);
    }

    #[test]
    fn user_tokens_bind_specific_person_id() {
        let store = test_store("anna:tok-a, marek:tok-m", None, false);
        assert!(store.has_user_tokens());
        assert_eq!(store.lookup_user_token("tok-a").as_deref(), Some("anna"));
        assert_eq!(store.lookup_user_token("tok-m").as_deref(), Some("marek"));
        assert_eq!(store.lookup_user_token("wrong"), None);
        assert!(store.verify_user_credential("anna", "tok-a"));
        assert!(!store.verify_user_credential("anna", "tok-m"));
    }

    #[test]
    fn trusted_proxy_header_never_accepts_x_person_id() {
        let store = test_store("", Some("X-Person-Id"), false);
        assert_eq!(store.trusted_proxy_header(), None);
        let store2 = test_store("", Some("X-Forwarded-User"), false);
        assert_eq!(store2.trusted_proxy_header(), Some("x-forwarded-user"));
    }

    #[test]
    fn extract_cookie_reads_session_token() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("other=1; alerthub_session=abc123; theme=dark"),
        );
        assert_eq!(
            extract_cookie(&h, SESSION_COOKIE_NAME).as_deref(),
            Some("abc123")
        );
        assert_eq!(extract_cookie(&h, "missing"), None);
    }
}
