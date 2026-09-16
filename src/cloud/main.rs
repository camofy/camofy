mod api;
mod auth;
mod oauth;
mod provider;
mod security;
mod store;
mod sync;
mod worker;

use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use sqlx::PgPool;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, Semaphore, broadcast};
use uuid::Uuid;

#[derive(Clone)]
pub struct App {
    pub db: PgPool,
    pub vault: security::Vault,
    pub origin: String,
    pub legacy_origins: Vec<String>,
    pub secure: bool,
    pub registration: bool,
    pub private_egress: bool,
    pub workers: usize,
    pub topics: Arc<Mutex<HashMap<Uuid, broadcast::Sender<()>>>>,
    pub hash_slots: Arc<Semaphore>,
}
impl App {
    pub fn accepts_origin(&self, value: &str) -> bool {
        value == self.origin || self.legacy_origins.iter().any(|v| v == value)
    }
}
#[derive(Debug)]
pub struct Error {
    status: StatusCode,
    message: String,
}
impl Error {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
    pub fn bad(s: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, s)
    }
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not found")
    }
    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "authentication required or invalid credentials",
        )
    }
    pub fn internal(&self) -> &str {
        &self.message
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let message = if self.status.is_server_error() {
            tracing::error!("cloud internal failure: {}", self.message);
            "internal server error".to_string()
        } else {
            self.message
        };
        (self.status, Json(json!({"error":message}))).into_response()
    }
}
macro_rules! internal_error {($($t:ty),*)=>{$(impl From<$t> for Error {fn from(e:$t)->Self{Self::new(StatusCode::INTERNAL_SERVER_ERROR,e.to_string())}})*};}
internal_error!(
    sqlx::Error,
    anyhow::Error,
    serde_json::Error,
    tokio::task::JoinError
);
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn router(app: App) -> Router {
    Router::new()
        .route(
            "/api/health",
            get(|| async { Json(json!({"status":"ok","service":"camofy-cloud"})) }),
        )
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/me", get(auth::me))
        .route("/api/auth/logout", post(auth::logout))
        .route(
            "/api/oauth/device_authorization",
            post(oauth::device_authorization),
        )
        .route("/api/oauth/token", post(oauth::token))
        .route("/api/oauth/requests/:code", get(oauth::request_info))
        .route("/api/oauth/approve", post(oauth::approve))
        .route("/api/resources", get(api::resources).post(api::create))
        .route(
            "/api/resources/:id",
            axum::routing::put(api::update).delete(api::delete),
        )
        .route("/api/profiles/:id/refresh", post(api::refresh))
        .route("/api/profiles/:id/content", get(api::profile_content))
        .route("/api/proxies/egress-preview", post(provider::preview))
        .route("/api/bundles/:id/revisions", get(api::revisions))
        .route("/api/bundles/:id/rollback", post(api::rollback))
        .route("/api/bundles/:id/preview/:format", get(api::preview))
        .route("/api/tokens", get(api::tokens).post(api::token))
        .route("/api/tokens/:id", axum::routing::delete(api::revoke))
        .route("/api/devices/:id/test", post(api::test_device))
        .route("/api/devices/:id/control", post(api::control_device))
        .route("/api/sync/desired", get(sync::desired))
        .route("/api/sync/report", post(sync::report))
        .route("/api/sync/revisions/:id/:format", get(sync::download))
        .route("/api/sync/ws", get(sync::socket))
        .route("/sub/:token", get(sync::identity_subscription))
        .route("/sub/:token/:format", get(sync::subscription))
        .nest_service(
            "/assets",
            tower_http::services::ServeDir::new("web/dist/assets"),
        )
        .fallback_service(tower_http::services::ServeFile::new("web/dist/index.html"))
        .layer(DefaultBodyLimit::max(5 * 1024 * 1024))
        .layer(
            tower_http::set_header::SetResponseHeaderLayer::if_not_present(
                header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            ),
        )
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            axum::http::HeaderValue::from_static("nosniff"),
        ))
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            axum::http::HeaderValue::from_static("no-referrer"),
        ))
        .with_state(app)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(30)
        .connect(&std::env::var("DATABASE_URL")?)
        .await?;
    sqlx::migrate!("./migrations").run(&db).await?;
    let origin = std::env::var("CAMOFY_PUBLIC_URL")
        .unwrap_or_else(|_| "http://localhost:3000".into())
        .trim_end_matches('/')
        .to_string();
    let parsed = url::Url::parse(&origin)?;
    anyhow::ensure!(
        ["http", "https"].contains(&parsed.scheme())
            && parsed.path() == "/"
            && parsed.query().is_none()
            && parsed.fragment().is_none(),
        "CAMOFY_PUBLIC_URL must be an origin without a path/query"
    );
    let legacy_origins = std::env::var("CAMOFY_LEGACY_ORIGINS")
        .unwrap_or_default()
        .split(',')
        .filter(|v| !v.trim().is_empty())
        .map(|v| {
            let v = v.trim().trim_end_matches('/');
            let u = url::Url::parse(v)?;
            anyhow::ensure!(
                u.scheme() == "https"
                    && u.username().is_empty()
                    && u.password().is_none()
                    && u.path() == "/"
                    && u.query().is_none()
                    && u.fragment().is_none(),
                "legacy origins must be HTTPS origins"
            );
            Ok(u.origin().ascii_serialization())
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let app = App {
        db,
        vault: security::Vault::new(&std::env::var("CAMOFY_ENCRYPTION_KEY")?)?,
        secure: origin.starts_with("https:"),
        origin,
        legacy_origins,
        registration: std::env::var("CAMOFY_REGISTRATION").as_deref() != Ok("false"),
        private_egress: std::env::var("CAMOFY_ALLOW_PRIVATE_EGRESS").as_deref() == Ok("true"),
        workers: std::env::var("CAMOFY_FETCH_WORKERS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8)
            .clamp(1, 64),
        topics: Default::default(),
        hash_slots: Arc::new(Semaphore::new(4)),
    };
    sync::listen(app.clone()).await;
    worker::start(app.clone()).await;
    let addr = std::env::var("CAMOFY_LISTEN").unwrap_or_else(|_| "0.0.0.0:3000".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("camofy-cloud listening at {addr}");
    axum::serve(
        listener,
        router(app).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
