//! Headless HTTP runtime.

use axum::{routing::get, Json, Router};
use serde::Serialize;
use std::net::SocketAddr;
use std::path::PathBuf;

use crate::state::paths::AppDataPaths;

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

pub fn router() -> Router {
    Router::new().route("/healthz", get(health))
}

pub async fn run() -> anyhow::Result<()> {
    let listen_addr = std::env::var("SOUNDROBE_LISTEN_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse::<SocketAddr>()?;
    let data_dir = std::env::var_os("SOUNDROBE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/config"));
    AppDataPaths::server(data_dir).prepare()?;

    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    axum::serve(listener, router()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_is_the_public_server_readiness_endpoint() {
        let response = router()
            .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/json"
        );
    }

    #[tokio::test]
    async fn unknown_routes_are_not_exposed() {
        let response = router()
            .oneshot(Request::builder().uri("/api").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
