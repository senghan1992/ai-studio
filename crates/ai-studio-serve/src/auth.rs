//! Bearer-token gate for the API.
//!
//! Hosting the server for anyone but yourself needs a lock on the door: without
//! one, `--host 0.0.0.0` hands every document on the machine to the network.
//! A token rides either as `Authorization: Bearer <token>` — the normal API
//! path — or as a `?token=` query parameter, because two request kinds cannot
//! carry headers: the export download (a plain navigation so the browser
//! handles `Content-Disposition`) and images loaded through `<img src>`.
//!
//! `--token` gives one write token; `--tokens <파일>` gives named tokens with a
//! role each, so a team can share a workspace where some members (or agents)
//! may only read.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Read,
    Write,
}

impl Role {
    fn parse(text: &str) -> Option<Role> {
        match text.trim() {
            "read" | "읽기" => Some(Role::Read),
            "write" | "쓰기" => Some(Role::Write),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Credential {
    pub name: String,
    pub role: Role,
    token: String,
}

#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    credentials: Vec<Credential>,
}

impl AuthConfig {
    pub fn is_empty(&self) -> bool {
        self.credentials.is_empty()
    }

    pub fn summary(&self) -> String {
        let write = self
            .credentials
            .iter()
            .filter(|c| c.role == Role::Write)
            .count();
        let read = self.credentials.len() - write;
        format!(
            "토큰 {}개 (쓰기 {write} · 읽기 {read})",
            self.credentials.len()
        )
    }

    /// One anonymous write token — the `--token` shape.
    pub fn single(token: String) -> AuthConfig {
        AuthConfig {
            credentials: vec![Credential {
                name: "token".into(),
                role: Role::Write,
                token,
            }],
        }
    }

    pub fn add_single(&mut self, token: String) {
        self.credentials.push(Credential {
            name: "token".into(),
            role: Role::Write,
            token,
        });
    }

    /// Parse a tokens file: one `이름:역할:토큰` per line, `#` for comments.
    /// Roles are `read`/`읽기` and `write`/`쓰기`.
    pub fn parse_file(text: &str) -> Result<AuthConfig, String> {
        let mut credentials = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.splitn(3, ':');
            let (name, role, token) = (parts.next(), parts.next(), parts.next());
            let (Some(name), Some(role), Some(token)) = (name, role, token) else {
                return Err(format!(
                    "{}번째 줄: `이름:역할:토큰` 형식이 아닙니다",
                    i + 1
                ));
            };
            let Some(role) = Role::parse(role) else {
                return Err(format!(
                    "{}번째 줄: 역할은 read/write(읽기/쓰기)여야 합니다: {role}",
                    i + 1
                ));
            };
            let token = token.trim();
            if name.trim().is_empty() || token.is_empty() {
                return Err(format!("{}번째 줄: 이름과 토큰은 비울 수 없습니다", i + 1));
            }
            credentials.push(Credential {
                name: name.trim().to_string(),
                role,
                token: token.to_string(),
            });
        }
        if credentials.is_empty() {
            return Err("토큰 파일에 항목이 없습니다".into());
        }
        Ok(AuthConfig { credentials })
    }

    /// The credential a presented token belongs to. Every entry is compared so
    /// the timing does not say which prefix was close.
    fn authenticate(&self, presented: &str) -> Option<&Credential> {
        let mut found = None;
        for credential in &self.credentials {
            if token_matches(&credential.token, presented) && found.is_none() {
                found = Some(credential);
            }
        }
        found
    }
}

/// True when `presented` equals `expected`, in time independent of where they
/// differ — a timing oracle on a secret is a slow way to publish it.
pub fn token_matches(expected: &str, presented: &str) -> bool {
    let a = expected.as_bytes();
    let b = presented.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The token a request presented, from the header or the query string.
pub fn presented_token(request: &Request<Body>) -> Option<String> {
    if let Some(value) = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(token) = value.strip_prefix("Bearer ") {
            return Some(token.trim().to_string());
        }
    }
    let query = request.uri().query()?;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "token").then(|| percent_decode(value))
    })
}

/// True for a request a read-only token may make: anything that writes nothing.
/// `POST /recalc` and `POST …/preview` are computations, not mutations.
pub fn readable(method: &Method, path: &str) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return true;
    }
    *method == Method::POST && (path.ends_with("/recalc") || path.ends_with("/preview"))
}

pub async fn require_token(
    config: Arc<AuthConfig>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let presented = presented_token(&request).unwrap_or_default();
    let Some(credential) = config.authenticate(&presented) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "접속 토큰이 필요합니다 — Authorization: Bearer <토큰> 또는 ?token=<토큰>"
            })),
        )
            .into_response();
    };
    if credential.role == Role::Read && !readable(request.method(), request.uri().path()) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": format!("읽기 전용 토큰입니다 ({}) — 이 요청에는 쓰기 권한이 필요합니다", credential.name)
            })),
        )
            .into_response();
    }
    next.run(request).await
}

/// Minimal percent-decoding for a query value; a token is URL-safe in practice
/// but a client that encodes it anyway should still get in.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).and_then(|b| (*b as char).to_digit(16)),
                bytes.get(i + 2).and_then(|b| (*b as char).to_digit(16)),
            ) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(uri: &str, auth: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri(uri);
        if let Some(value) = auth {
            builder = builder.header("authorization", value);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[test]
    fn the_comparison_accepts_only_the_exact_token() {
        assert!(token_matches("secret", "secret"));
        assert!(!token_matches("secret", "secre"));
        assert!(!token_matches("secret", "secreT"));
        assert!(!token_matches("secret", ""));
    }

    #[test]
    fn a_token_arrives_by_header_or_query() {
        let by_header = request("/api/projects", Some("Bearer abc123"));
        assert_eq!(presented_token(&by_header).as_deref(), Some("abc123"));

        let by_query = request("/api/projects/x/export/xlsx?token=abc123", None);
        assert_eq!(presented_token(&by_query).as_deref(), Some("abc123"));

        let encoded = request("/api/x?path=a.png&token=a%2Bb", None);
        assert_eq!(presented_token(&encoded).as_deref(), Some("a+b"));

        let none = request("/api/projects", None);
        assert_eq!(presented_token(&none), None);

        // Basic auth is not a bearer token.
        let basic = request("/api/projects", Some("Basic abc123"));
        assert_eq!(presented_token(&basic), None);
    }

    #[test]
    fn a_tokens_file_names_each_credential() {
        let config =
            AuthConfig::parse_file("# 팀\n지민:write:aaa\n리뷰봇:read:bbb\n\n인덱서:읽기:ccc\n")
                .unwrap();
        assert_eq!(config.credentials.len(), 3);
        assert_eq!(config.authenticate("aaa").unwrap().name, "지민");
        assert_eq!(config.authenticate("aaa").unwrap().role, Role::Write);
        assert_eq!(config.authenticate("ccc").unwrap().role, Role::Read);
        assert!(config.authenticate("zzz").is_none());

        assert!(AuthConfig::parse_file("고장난 줄").is_err());
        assert!(AuthConfig::parse_file("이름:admin:토큰").is_err());
        assert!(AuthConfig::parse_file("# 주석뿐\n").is_err());
    }

    #[test]
    fn a_read_role_reads_and_computes_but_never_writes() {
        let get = Method::GET;
        let post = Method::POST;
        let put = Method::PUT;
        let delete = Method::DELETE;
        assert!(readable(&get, "/projects"));
        assert!(readable(&get, "/projects/x/export/xlsx"));
        assert!(readable(&post, "/recalc"));
        assert!(readable(&post, "/projects/x/preview"));
        assert!(!readable(&put, "/projects/x"));
        assert!(!readable(&post, "/projects"));
        assert!(!readable(&post, "/projects/x/restore"));
        assert!(!readable(&post, "/import"));
        assert!(!readable(&delete, "/projects/x"));
    }
}
