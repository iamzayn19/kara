//! Use inference from another Kara machine you own.
//!
//! On the machine with the compute: `kara serve --inference` exposes its
//! inference session over an OpenAI-compatible HTTP API protected by a bearer
//! token. It binds to `127.0.0.1` unless an address is given explicitly.
//!
//! On the client: `kara connect <url> --token <token>` checks the server,
//! stores the token in an owner-only file and points `[inference]` at it.
//! The rest of Kara sees an ordinary [`crate::InferenceProvider`].
//!
//! Traffic is plain HTTP. Use it on a network you trust (LAN, VPN such as
//! WireGuard/Tailscale) or through an SSH tunnel.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use futures_util::TryStreamExt;
use kara_core::config_edit;
use kara_core::KaraPaths;
use std::net::SocketAddr;
use std::sync::Arc;

pub const DEFAULT_PORT: u16 = 7878;

/// Generate a random 256-bit token as hex.
pub fn new_token() -> String {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// The serving token, created on first use (owner-only file).
pub fn serve_token(paths: &KaraPaths) -> anyhow::Result<String> {
    let f = paths.credentials_dir().join("serve.token");
    if let Ok(t) = std::fs::read_to_string(&f) {
        let t = t.trim().to_string();
        if t.len() >= 32 {
            return Ok(t);
        }
    }
    let t = new_token();
    kara_core::paths::write_private(&f, t.as_bytes())?;
    Ok(t)
}

/// Normalize `host`, `host:port` or a URL into an OpenAI-compatible base URL.
pub fn normalize_url(input: &str) -> String {
    let mut u = input.trim().trim_end_matches('/').to_string();
    if !u.contains("://") {
        u = format!("http://{u}");
    }
    let has_port = u
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .map(|auth| {
            auth.rsplit_once(':')
                .map(|(_, p)| p.parse::<u16>().is_ok())
                .unwrap_or(false)
        })
        .unwrap_or(false);
    if !has_port {
        let (scheme, rest) = u.split_once("://").expect("scheme added above");
        let (auth, path) = rest
            .split_once('/')
            .map(|(a, p)| (a, format!("/{p}")))
            .unwrap_or((rest, String::new()));
        u = format!("{scheme}://{auth}:{DEFAULT_PORT}{path}");
    }
    if !u.ends_with("/v1") {
        u.push_str("/v1");
    }
    u
}

/// Check a Kara server and configure this machine to use it.
pub async fn connect(paths: &KaraPaths, url: &str, token: &str) -> anyhow::Result<String> {
    let base = normalize_url(url);
    let root = base.trim_end_matches("/v1");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let health = client
        .get(format!("{root}/health"))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("cannot reach {root}: {e}"))?;
    if !health.status().is_success() {
        anyhow::bail!("{root}/health returned {}", health.status());
    }
    let models = client
        .get(format!("{base}/models"))
        .bearer_auth(token)
        .send()
        .await?;
    if models.status() == StatusCode::UNAUTHORIZED {
        anyhow::bail!("the token was rejected by {root}");
    }
    if !models.status().is_success() {
        anyhow::bail!("{base}/models returned {}", models.status());
    }
    let token_file = paths.credentials_dir().join("remote.token");
    kara_core::paths::write_private(&token_file, token.as_bytes())?;
    let cfg = paths.config_file();
    config_edit::set_in_file(&cfg, "inference.provider", "kara")?;
    config_edit::set_in_file(&cfg, "inference.endpoint", &base)?;
    config_edit::set_in_file(
        &cfg,
        "inference.api_key_file",
        &token_file.display().to_string(),
    )?;
    config_edit::set_in_file(&cfg, "inference.model", "")?;
    Ok(base)
}

struct Proxy {
    upstream: String,
    upstream_key: Option<String>,
    token: String,
    client: reqwest::Client,
}

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    let Some(v) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some(given) = v.strip_prefix("Bearer ") else {
        return false;
    };
    // Constant-time comparison.
    let (a, b) = (given.trim().as_bytes(), token.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn forward(proxy: Arc<Proxy>, path: &str, req: Request<Body>) -> Response {
    if !authorized(req.headers(), &proxy.token) {
        return (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response();
    }
    let method = req.method().clone();
    let body = match axum::body::to_bytes(req.into_body(), 64 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "request too large").into_response(),
    };
    let mut up = proxy
        .client
        .request(method, format!("{}{path}", proxy.upstream))
        .header(header::CONTENT_TYPE, "application/json")
        .body(body);
    if let Some(k) = &proxy.upstream_key {
        up = up.bearer_auth(k);
    }
    match up.send().await {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let ctype = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/json")
                .to_string();
            let stream = resp.bytes_stream().map_err(std::io::Error::other);
            Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, ctype)
                .body(Body::from_stream(stream))
                .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            format!("inference backend unavailable: {e}"),
        )
            .into_response(),
    }
}

/// Router for `kara serve --inference`.
pub fn router(upstream: &str, upstream_key: Option<String>, token: String) -> Router {
    let proxy = Arc::new(Proxy {
        upstream: upstream.trim_end_matches('/').to_string(),
        upstream_key,
        token,
        client: reqwest::Client::new(),
    });
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route(
            "/v1/models",
            get(
                |State(p): State<Arc<Proxy>>, req: Request<Body>| async move {
                    forward(p, "/models", req).await
                },
            ),
        )
        .route(
            "/v1/chat/completions",
            post(
                |State(p): State<Arc<Proxy>>, req: Request<Body>| async move {
                    forward(p, "/chat/completions", req).await
                },
            ),
        )
        .with_state(proxy)
}

/// Serve until the future returned is dropped or the process exits.
pub async fn serve(
    listen: SocketAddr,
    upstream: &str,
    upstream_key: Option<String>,
    token: String,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    axum::serve(listener, router(upstream, upstream_key, token)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::OpenAiCompatProvider;
    use crate::{ChatRequest, InferenceProvider, Message};
    use tokio_util::sync::CancellationToken;

    #[test]
    fn url_normalization() {
        assert_eq!(normalize_url("192.168.1.20"), "http://192.168.1.20:7878/v1");
        assert_eq!(normalize_url("box.local:9000"), "http://box.local:9000/v1");
        assert_eq!(normalize_url("http://box:7878/v1/"), "http://box:7878/v1");
        assert_eq!(
            normalize_url("https://gpu.example"),
            "https://gpu.example:7878/v1"
        );
    }

    #[test]
    fn token_comparison() {
        let mut h = HeaderMap::new();
        assert!(!authorized(&h, "abc"));
        h.insert(header::AUTHORIZATION, "Bearer abc".parse().unwrap());
        assert!(authorized(&h, "abc"));
        h.insert(header::AUTHORIZATION, "Bearer abd".parse().unwrap());
        assert!(!authorized(&h, "abc"));
        h.insert(header::AUTHORIZATION, "Bearer ab".parse().unwrap());
        assert!(!authorized(&h, "abc"));
    }

    /// A fake OpenAI-compatible upstream.
    async fn upstream() -> String {
        let app = Router::new()
            .route("/v1/models", get(|| async { axum::Json(serde_json::json!({"data": [{"id": "fake-model"}]})) }))
            .route(
                "/v1/chat/completions",
                post(|| async {
                    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hello from far away\"}}]}\n\ndata: [DONE]\n\n";
                    ([(header::CONTENT_TYPE, "text/event-stream")], body)
                }),
            );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        format!("http://{addr}/v1")
    }

    #[tokio::test]
    async fn remote_kara_round_trip_with_token() {
        let up = upstream().await;
        let token = new_token();
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let app = router(&up, None, token.clone());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

        // Without the token: rejected.
        let r = reqwest::get(format!("http://{addr}/v1/models"))
            .await
            .unwrap();
        assert_eq!(r.status(), 401);

        // `kara connect` writes config and an owner-only token file.
        let d = tempfile::tempdir().unwrap();
        let paths = KaraPaths::at(d.path());
        let base = connect(&paths, &format!("http://{addr}"), &token)
            .await
            .unwrap();
        let cfg = kara_core::Config::load_user(&paths.config_file()).unwrap();
        assert_eq!(
            cfg.inference.provider,
            kara_core::config::ProviderKind::Kara
        );
        assert_eq!(cfg.inference.endpoint, base);
        assert!(connect(&paths, &format!("http://{addr}"), "wrong")
            .await
            .is_err());

        // The provider streams through the proxy like any other.
        let p = OpenAiCompatProvider::new(&base, "fake-model").with_api_key(Some(token));
        let r = p
            .chat(
                ChatRequest {
                    messages: vec![Message::user("hi")],
                    ..Default::default()
                },
                &|_| {},
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(r.content, "hello from far away");
    }

    #[test]
    fn serve_token_is_stable_and_private() {
        let d = tempfile::tempdir().unwrap();
        let paths = KaraPaths::at(d.path());
        let a = serve_token(&paths).unwrap();
        let b = serve_token(&paths).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }
}
