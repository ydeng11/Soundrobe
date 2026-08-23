//! Headless HTTP runtime.

use axum::{
    extract::{rejection::JsonRejection, DefaultBodyLimit, Json, State},
    http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde::Serialize;
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex, RwLock};
use url::Url;
use uuid::Uuid;

use crate::state::paths::AppDataPaths;

const SESSION_COOKIE: &str = "soundrobe_session";
const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const DEFAULT_LOGIN_WINDOW: Duration = Duration::from_secs(60);
const DEFAULT_MAX_FAILED_LOGINS: u32 = 5;

#[derive(Clone)]
pub struct ServerConfig {
    listen_addr: SocketAddr,
    data_dir: PathBuf,
    auth: AuthConfig,
    session_ttl: Duration,
    login_window: Duration,
    max_failed_logins: u32,
}

impl ServerConfig {
    fn from_env() -> anyhow::Result<Self> {
        let listen_addr = std::env::var("SOUNDROBE_LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
            .parse::<SocketAddr>()?;
        let data_dir = std::env::var_os("SOUNDROBE_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/config"));
        let password_file = std::env::var_os("SOUNDROBE_AUTH_PASSWORD_FILE").map(PathBuf::from);
        let auth = AuthConfig::from_sources(
            password_file.as_deref(),
            std::env::var("SOUNDROBE_AUTH_PASSWORD").ok().as_deref(),
            std::env::var("SOUNDROBE_PUBLIC_URL").ok().as_deref(),
        )?;

        Ok(Self {
            listen_addr,
            data_dir,
            auth,
            session_ttl: DEFAULT_SESSION_TTL,
            login_window: DEFAULT_LOGIN_WINDOW,
            max_failed_logins: DEFAULT_MAX_FAILED_LOGINS,
        })
    }

    #[cfg(test)]
    fn for_tests(password: &str, public_url: &str) -> Self {
        Self {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            data_dir: PathBuf::from("/tmp/soundrobe-test-config"),
            auth: AuthConfig::from_sources(None, Some(password), Some(public_url)).unwrap(),
            session_ttl: DEFAULT_SESSION_TTL,
            login_window: DEFAULT_LOGIN_WINDOW,
            max_failed_logins: DEFAULT_MAX_FAILED_LOGINS,
        }
    }
}

#[derive(Clone)]
struct AuthConfig {
    password: Arc<str>,
    public_origin: Arc<str>,
    secure_cookie: bool,
}

impl AuthConfig {
    fn from_sources(
        password_file: Option<&Path>,
        fallback_password: Option<&str>,
        public_url: Option<&str>,
    ) -> anyhow::Result<Self> {
        let public_url =
            public_url.ok_or_else(|| anyhow::anyhow!("SOUNDROBE_PUBLIC_URL is required"))?;
        let (public_origin, secure_cookie) = parse_public_origin(public_url)?;
        let password = match password_file {
            Some(path) => std::fs::read_to_string(path)
                .map(|value| value.trim_end_matches(['\r', '\n']).to_string())
                .map_err(|error| {
                    anyhow::anyhow!("unable to read SOUNDROBE_AUTH_PASSWORD_FILE: {error}")
                })?,
            None => fallback_password.unwrap_or_default().to_string(),
        };
        if password.is_empty() {
            return Err(anyhow::anyhow!(
                "SOUNDROBE_AUTH_PASSWORD_FILE or SOUNDROBE_AUTH_PASSWORD is required"
            ));
        }

        Ok(Self {
            password: Arc::from(password),
            public_origin: Arc::from(public_origin),
            secure_cookie,
        })
    }

    fn password_matches(&self, candidate: &str) -> bool {
        self.password.as_bytes().ct_eq(candidate.as_bytes()).into()
    }
}

fn parse_public_origin(public_url: &str) -> anyhow::Result<(String, bool)> {
    let url = Url::parse(public_url)
        .map_err(|_| anyhow::anyhow!("SOUNDROBE_PUBLIC_URL must be an absolute HTTP(S) origin"))?;
    let scheme = url.scheme();
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(anyhow::anyhow!(
            "SOUNDROBE_PUBLIC_URL must use http or https"
        ));
    }
    if url.host_str().is_none() {
        return Err(anyhow::anyhow!("SOUNDROBE_PUBLIC_URL must include a host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(anyhow::anyhow!(
            "SOUNDROBE_PUBLIC_URL must not include credentials"
        ));
    }
    if url.path() != "" && url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(anyhow::anyhow!(
            "SOUNDROBE_PUBLIC_URL must contain only an origin"
        ));
    }

    Ok((
        url.origin().ascii_serialization(),
        scheme.eq_ignore_ascii_case("https"),
    ))
}

#[derive(Clone)]
struct ServerState {
    auth: AuthService,
}

#[derive(Clone)]
struct AuthService {
    config: AuthConfig,
    sessions: Arc<RwLock<HashMap<String, Instant>>>,
    failed_logins: Arc<Mutex<FailedLogins>>,
    session_ttl: Duration,
    login_window: Duration,
    max_failed_logins: u32,
}

struct FailedLogins {
    window_started: Instant,
    count: u32,
}

enum LoginAttempt {
    Authenticated,
    Invalid,
    Throttled,
}

impl AuthService {
    fn new(config: &ServerConfig) -> Self {
        Self {
            config: config.auth.clone(),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            failed_logins: Arc::new(Mutex::new(FailedLogins {
                window_started: Instant::now(),
                count: 0,
            })),
            session_ttl: config.session_ttl,
            login_window: config.login_window,
            max_failed_logins: config.max_failed_logins,
        }
    }

    async fn verify_password(&self, candidate: &str) -> LoginAttempt {
        let mut failed_logins = self.failed_logins.lock().await;
        if failed_logins.window_started.elapsed() >= self.login_window {
            failed_logins.window_started = Instant::now();
            failed_logins.count = 0;
        }
        if failed_logins.count >= self.max_failed_logins {
            return LoginAttempt::Throttled;
        }
        if self.config.password_matches(candidate) {
            failed_logins.window_started = Instant::now();
            failed_logins.count = 0;
            LoginAttempt::Authenticated
        } else {
            failed_logins.count = failed_logins.count.saturating_add(1);
            LoginAttempt::Invalid
        }
    }

    async fn create_session(&self) -> String {
        let token = Uuid::new_v4().simple().to_string();
        self.sessions
            .write()
            .await
            .insert(token.clone(), Instant::now() + self.session_ttl);
        token
    }

    async fn has_valid_session(&self, token: Option<&str>) -> bool {
        let Some(token) = token else {
            return false;
        };
        let mut sessions = self.sessions.write().await;
        let Some(expires_at) = sessions.get(token).copied() else {
            return false;
        };
        if expires_at <= Instant::now() {
            sessions.remove(token);
            return false;
        }
        true
    }

    async fn remove_session(&self, token: Option<&str>) {
        if let Some(token) = token {
            self.sessions.write().await.remove(token);
        }
    }

    fn session_cookie(&self, token: &str) -> String {
        let secure = if self.config.secure_cookie {
            "; Secure"
        } else {
            ""
        };
        format!(
            "{SESSION_COOKIE}={token}; Path=/; Max-Age={}; HttpOnly; SameSite=Strict{secure}",
            self.session_ttl.as_secs()
        )
    }

    fn clear_session_cookie(&self) -> String {
        let secure = if self.config.secure_cookie {
            "; Secure"
        } else {
            ""
        };
        format!("{SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict{secure}")
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

#[derive(Deserialize)]
struct LoginRequest {
    password: String,
}

#[derive(Serialize)]
struct SessionResponse {
    authenticated: bool,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

fn error_response(status: StatusCode, message: &'static str) -> Response {
    (status, Json(ErrorResponse { error: message })).into_response()
}

fn normalize_error_response(mut response: Response) -> Response {
    let status = response.status();
    let Some(message) = (match status {
        StatusCode::NOT_FOUND => Some("not found"),
        StatusCode::METHOD_NOT_ALLOWED => Some("method not allowed"),
        StatusCode::INTERNAL_SERVER_ERROR => Some("internal server error"),
        _ => None,
    }) else {
        return response;
    };
    let allow = response.headers().get(header::ALLOW).cloned();
    response = error_response(status, message);
    if let Some(allow) = allow {
        response.headers_mut().insert(header::ALLOW, allow);
    }
    response
}

fn normalized_origin(value: &str) -> Option<String> {
    parse_public_origin(value).ok().map(|(origin, _)| origin)
}

fn is_api_mutation(method: &Method, path: &str) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) && path.starts_with("/api/v1/")
}

fn is_public_auth_endpoint(path: &str) -> bool {
    matches!(
        path,
        "/api/v1/auth/login" | "/api/v1/auth/logout" | "/api/v1/auth/session"
    )
}

async fn enforce_web_security(
    State(state): State<ServerState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if is_api_mutation(request.method(), path) {
        let origin_matches = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            .and_then(normalized_origin)
            .is_some_and(|origin| origin == state.auth.config.public_origin.as_ref());
        if !origin_matches {
            return error_response(StatusCode::FORBIDDEN, "same-origin request required");
        }
    }
    if path.starts_with("/api/v1/")
        && !is_public_auth_endpoint(path)
        && !state
            .auth
            .has_valid_session(session_token(request.headers()))
            .await
    {
        return error_response(StatusCode::UNAUTHORIZED, "authentication required");
    }
    next.run(request).await
}

async fn security_headers(
    State(state): State<ServerState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let mut response = normalize_error_response(next.run(request).await);
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'",
        ),
    );
    headers.insert(
        header::HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        header::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), geolocation=(), microphone=()"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if state.auth.config.secure_cookie {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
    response
}

fn session_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (name, value) = cookie.trim().split_once('=')?;
                (name == SESSION_COOKIE && !value.is_empty()).then_some(value)
            })
        })
}

async fn login(
    State(state): State<ServerState>,
    payload: Result<Json<LoginRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => {
            if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                return error_response(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
            }
            return error_response(StatusCode::BAD_REQUEST, "invalid login request");
        }
    };
    match state.auth.verify_password(&payload.password).await {
        LoginAttempt::Throttled => {
            return error_response(StatusCode::TOO_MANY_REQUESTS, "login temporarily blocked")
        }
        LoginAttempt::Invalid => {
            return error_response(StatusCode::UNAUTHORIZED, "invalid credentials")
        }
        LoginAttempt::Authenticated => {}
    }

    let token = state.auth.create_session().await;
    let mut response = Json(SessionResponse {
        authenticated: true,
    })
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::try_from(state.auth.session_cookie(&token)).unwrap(),
    );
    response
}

async fn session(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    let authenticated = state.auth.has_valid_session(session_token(&headers)).await;
    Json(SessionResponse { authenticated }).into_response()
}

async fn logout(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    state.auth.remove_session(session_token(&headers)).await;
    let mut response = Json(SessionResponse {
        authenticated: false,
    })
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::try_from(state.auth.clear_session_cookie()).unwrap(),
    );
    response
}

pub fn router(config: ServerConfig) -> Router {
    let state = ServerState {
        auth: AuthService::new(&config),
    };
    let login_route = Router::new()
        .route("/api/v1/auth/login", post(login))
        .layer(DefaultBodyLimit::max(64 * 1024));

    Router::new()
        .route("/healthz", get(health))
        .route("/api/v1/auth/session", get(session))
        .route("/api/v1/auth/logout", post(logout))
        .merge(login_route)
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            enforce_web_security,
        ))
        .layer(middleware::from_fn_with_state(state, security_headers))
}

pub async fn run() -> anyhow::Result<()> {
    let config = ServerConfig::from_env()?;
    AppDataPaths::server(config.data_dir.clone()).prepare()?;

    let listener = tokio::net::TcpListener::bind(config.listen_addr).await?;
    axum::serve(listener, router(config)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use std::time::Duration;
    use tower::ServiceExt;

    fn test_config() -> ServerConfig {
        ServerConfig::for_tests("correct horse battery staple", "https://soundrobe.test")
    }

    fn origin_request(request: Request<Body>) -> Request<Body> {
        origin_request_with(request, "https://soundrobe.test")
    }

    fn origin_request_with(request: Request<Body>, origin: &str) -> Request<Body> {
        let (mut parts, body) = request.into_parts();
        parts
            .headers
            .insert(header::ORIGIN, origin.parse().unwrap());
        Request::from_parts(parts, body)
    }

    async fn login(router: Router, password: &str) -> axum::response::Response {
        login_with_origin(router, password, "https://soundrobe.test").await
    }

    async fn login_with_origin(
        router: Router,
        password: &str,
        origin: &str,
    ) -> axum::response::Response {
        router
            .oneshot(origin_request_with(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(r#"{{"password":"{password}"}}"#)))
                    .unwrap(),
                origin,
            ))
            .await
            .unwrap()
    }

    async fn assert_json_error(response: axum::response::Response, status: StatusCode, body: &str) {
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let actual = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(actual.as_ref(), body.as_bytes());
    }

    #[tokio::test]
    async fn health_is_the_public_server_readiness_endpoint() {
        let response = router(test_config())
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/json"
        );
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
        assert_eq!(
            response.headers()["permissions-policy"],
            "camera=(), geolocation=(), microphone=()"
        );
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            response.headers()[header::CONTENT_SECURITY_POLICY],
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'"
        );
        assert_eq!(
            response.headers()[header::STRICT_TRANSPORT_SECURITY],
            "max-age=31536000; includeSubDomains"
        );
        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
    }

    #[tokio::test]
    async fn unknown_routes_are_not_exposed() {
        let response = router(test_config())
            .oneshot(Request::builder().uri("/api").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_json_error(response, StatusCode::NOT_FOUND, r#"{"error":"not found"}"#).await;
    }

    #[tokio::test]
    async fn api_routes_require_a_session_except_for_auth_endpoints() {
        let response = router(test_config())
            .oneshot(
                Request::builder()
                    .uri("/api/v1/libraries")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::UNAUTHORIZED,
            r#"{"error":"authentication required"}"#,
        )
        .await;
    }

    #[test]
    fn startup_requires_public_origin_and_password() {
        let missing_password = AuthConfig::from_sources(None, None, Some("https://soundrobe.test"));
        assert!(missing_password.is_err());

        let missing_origin = AuthConfig::from_sources(None, Some("secret"), None);
        assert!(missing_origin.is_err());
        assert_eq!(
            parse_public_origin("https://soundrobe.test/").unwrap(),
            ("https://soundrobe.test".to_string(), true)
        );
        assert_eq!(
            parse_public_origin("http://soundrobe.test").unwrap(),
            ("http://soundrobe.test".to_string(), false)
        );
        assert_eq!(
            parse_public_origin("HTTPS://SOUNDROBE.TEST:443/").unwrap(),
            ("https://soundrobe.test".to_string(), true)
        );
        assert!(parse_public_origin("https://soundrobe.test/app").is_err());
    }

    #[tokio::test]
    async fn normalized_same_origin_headers_are_accepted() {
        let config = ServerConfig::for_tests(
            "correct horse battery staple",
            "HTTPS://SOUNDROBE.TEST:443/",
        );
        assert_eq!(
            login_with_origin(
                router(config),
                "correct horse battery staple",
                "HTTPS://SOUNDROBE.TEST:443",
            )
            .await
            .status(),
            StatusCode::OK
        );
    }

    #[test]
    fn password_file_takes_precedence_and_removes_only_trailing_newlines() {
        let path = std::env::temp_dir().join(format!("soundrobe-auth-{}", Uuid::new_v4()));
        std::fs::write(&path, "file-secret  \n").unwrap();

        let config = AuthConfig::from_sources(
            Some(&path),
            Some("environment-secret"),
            Some("https://soundrobe.test"),
        )
        .unwrap();

        std::fs::remove_file(path).unwrap();
        assert!(config.password_matches("file-secret  "));
        assert!(!config.password_matches("environment-secret"));
    }

    #[test]
    fn unreadable_password_file_fails_startup_instead_of_using_the_fallback() {
        let path = std::env::temp_dir().join(format!("soundrobe-missing-auth-{}", Uuid::new_v4()));
        let result = AuthConfig::from_sources(
            Some(&path),
            Some("environment-secret"),
            Some("https://soundrobe.test"),
        );

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn login_sets_opaque_strict_session_cookie_without_returning_the_password() {
        let response = login(router(test_config()), "correct horse battery staple").await;

        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.starts_with("soundrobe_session="));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("; Secure"));
        assert!(cookie.contains("Max-Age=86400"));
        assert!(!cookie.contains("correct horse"));
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("correct horse"));
    }

    #[tokio::test]
    async fn session_endpoint_reports_authenticated_cookie_and_logout_expires_it() {
        let app = router(test_config());
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        let cookie_pair = cookie.split(';').next().unwrap();

        let session_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/session")
                    .header(header::COOKIE, cookie_pair)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(session_response.status(), StatusCode::OK);
        let session_body = axum::body::to_bytes(session_response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(session_body.as_ref(), b"{\"authenticated\":true}");

        let logout_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/logout")
                    .header(header::COOKIE, cookie_pair)
                    .body(Body::empty())
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(logout_response.status(), StatusCode::OK);
        assert!(logout_response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Max-Age=0"));
        let session_after_logout = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/session")
                    .header(header::COOKIE, cookie_pair)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(session_after_logout.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"{\"authenticated\":false}");
        let fresh_session = router(test_config())
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/session")
                    .header(header::COOKIE, cookie_pair)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let fresh_body = axum::body::to_bytes(fresh_session.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(fresh_body.as_ref(), b"{\"authenticated\":false}");
    }

    #[tokio::test]
    async fn expired_sessions_are_rejected_without_exposing_session_state() {
        let mut config = test_config();
        config.session_ttl = Duration::from_millis(1);
        let app = router(config);
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/session")
                    .header(header::COOKIE, cookie.split(';').next().unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"{\"authenticated\":false}");
        let protected_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/libraries")
                    .header(header::COOKIE, cookie.split(';').next().unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_json_error(
            protected_response,
            StatusCode::UNAUTHORIZED,
            r#"{"error":"authentication required"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn failed_logins_are_rate_limited_and_success_resets_the_window() {
        let mut config = test_config();
        config.max_failed_logins = 2;
        config.login_window = Duration::from_secs(60);
        let app = router(config);

        assert_json_error(
            login(app.clone(), "wrong").await,
            StatusCode::UNAUTHORIZED,
            r#"{"error":"invalid credentials"}"#,
        )
        .await;
        assert_eq!(
            login(app.clone(), "correct horse battery staple")
                .await
                .status(),
            StatusCode::OK
        );
        assert_json_error(
            login(app.clone(), "wrong").await,
            StatusCode::UNAUTHORIZED,
            r#"{"error":"invalid credentials"}"#,
        )
        .await;
        assert_json_error(
            login(app.clone(), "wrong").await,
            StatusCode::UNAUTHORIZED,
            r#"{"error":"invalid credentials"}"#,
        )
        .await;
        assert_json_error(
            login(app, "wrong").await,
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":"login temporarily blocked"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn concurrent_failed_logins_share_one_atomic_throttle() {
        let mut config = test_config();
        config.max_failed_logins = 1;
        let app = router(config);
        let (first, second) = tokio::join!(login(app.clone(), "wrong"), login(app, "wrong"));
        let statuses = [first.status(), second.status()];

        assert!(statuses.contains(&StatusCode::UNAUTHORIZED));
        assert!(statuses.contains(&StatusCode::TOO_MANY_REQUESTS));
    }

    #[tokio::test]
    async fn mutation_requests_require_the_configured_same_origin() {
        let app = router(test_config());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/scan")
                    .header(header::ORIGIN, "https://evil.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::FORBIDDEN,
            r#"{"error":"same-origin request required"}"#,
        )
        .await;

        let missing_origin = router(test_config())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/scan")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_json_error(
            missing_origin,
            StatusCode::FORBIDDEN,
            r#"{"error":"same-origin request required"}"#,
        )
        .await;

        let accepted_app = router(test_config());
        let login_response = login(accepted_app.clone(), "correct horse battery staple").await;
        let cookie_pair = login_response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let accepted = accepted_app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/logout")
                    .header(header::COOKIE, cookie_pair)
                    .body(Body::empty())
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn malformed_login_requests_return_stable_json_errors() {
        let response = router(test_config())
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("not-json"))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid login request"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn oversized_login_requests_return_payload_too_large() {
        let oversized_password = "x".repeat(64 * 1024);
        let response = router(test_config())
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        r#"{{"password":"{oversized_password}"}}"#
                    )))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::PAYLOAD_TOO_LARGE,
            r#"{"error":"request body too large"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn method_failures_use_the_stable_json_error_contract() {
        let response = router(test_config())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::METHOD_NOT_ALLOWED,
            r#"{"error":"method not allowed"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn internal_failures_use_the_stable_json_error_contract() {
        let state = ServerState {
            auth: AuthService::new(&test_config()),
        };
        let app = Router::new()
            .route("/boom", get(|| async { StatusCode::INTERNAL_SERVER_ERROR }))
            .with_state(state.clone())
            .layer(middleware::from_fn_with_state(state, security_headers));
        let response = app
            .oneshot(Request::builder().uri("/boom").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":"internal server error"}"#,
        )
        .await;
    }
}
