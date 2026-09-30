//! Tests for serving the built frontend: cache headers, the SPA fallback to
//! `index.html`, and that an unknown `/api/...` path still gets a typed 404 rather
//! than falling through to `index.html`. Uses its own hand-built static directory
//! (never the real `frontend/dist`), per CLAUDE.md's fixture conventions elsewhere in
//! this file.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use axum::http::{StatusCode, header};
use engine::LimitsConfig;
use tower::ServiceExt;

mod support;
use support::{get, json_body, repo_root, server_config, test_router_with_config};

/// Writes a minimal built-frontend directory: `index.html` at the root and one hashed
/// asset under `assets/`, matching Vite's own output shape closely enough for the
/// static service's `/assets/` cache-header rule to apply.
fn write_static_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>spa-root</html>").unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(
        dir.path().join("assets").join("app-abc123.js"),
        "console.log('hi');",
    )
    .unwrap();
    dir
}

fn router_over(static_dir: std::path::PathBuf) -> (axum::Router, tempfile::TempDir) {
    let mut config = server_config();
    config.static_dir = static_dir;
    let limits = LimitsConfig::load(&repo_root().join("config/limits.toml")).unwrap();
    test_router_with_config(config, limits, Duration::from_millis(20))
}

#[tokio::test]
async fn index_html_is_served_with_no_cache() {
    let static_dir = write_static_dir();
    let (router, _engine_dir) = router_over(static_dir.path().to_path_buf());

    let response = router.oneshot(get("/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-cache"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body, "<html>spa-root</html>".as_bytes());
}

#[tokio::test]
async fn hashed_asset_is_served_immutable() {
    let static_dir = write_static_dir();
    let (router, _engine_dir) = router_over(static_dir.path().to_path_buf());

    let response = router.oneshot(get("/assets/app-abc123.js")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn unknown_client_route_falls_back_to_index_html() {
    let static_dir = write_static_dir();
    let (router, _engine_dir) = router_over(static_dir.path().to_path_buf());

    let response = router.oneshot(get("/portfolio/settings")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-cache"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body, "<html>spa-root</html>".as_bytes());
}

#[tokio::test]
async fn unknown_api_route_is_a_typed_404_not_index_html() {
    let static_dir = write_static_dir();
    let (router, _engine_dir) = router_over(static_dir.path().to_path_buf());

    let response = router.oneshot(get("/api/does-not-exist")).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn real_api_routes_still_win_over_the_static_fallback() {
    let static_dir = write_static_dir();
    let (router, _engine_dir) = router_over(static_dir.path().to_path_buf());

    let response = router.oneshot(get("/api/health")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn missing_static_dir_404s_instead_of_panicking() {
    let (router, _engine_dir) = router_over(std::path::PathBuf::from(
        "/nonexistent-rrm-static-dir-for-tests",
    ));

    let response = router.oneshot(get("/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
