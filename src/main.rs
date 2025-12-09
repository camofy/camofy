use axum::{
    http::{header, StatusCode, Uri},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use mime_guess::mime;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

mod app;
mod auth;
mod core;
mod logs;
mod subscriptions;
mod user_profiles;
mod mihomo;
mod geoip;
mod scheduler;

use crate::app::AppState;

#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Serialize)]
struct ApiResponse<T>
where
    T: Serialize,
{
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
enum ProfileType {
    Remote,
    User,
}

#[derive(Serialize, Deserialize, Clone)]
struct ProfileMeta {
    id: String,
    name: String,
    #[serde(rename = "profile_type")]
    profile_type: ProfileType,
    /// Path relative to <DATA_ROOT>/config, e.g. "subscriptions/<id>/subscription.yaml" or "user-profiles/<id>.yaml"
    path: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    last_fetch_time: Option<String>,
    #[serde(default)]
    last_fetch_status: Option<String>,
    #[serde(default)]
    last_modified_time: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct ScheduledTaskConfig {
    /// 类 crontab 表达式，形如 "0 3 * * *"
    cron: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    last_run_time: Option<String>,
    #[serde(default)]
    last_run_status: Option<String>,
    #[serde(default)]
    last_run_message: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct AppConfig {
    #[serde(default)]
    profiles: Vec<ProfileMeta>,
    #[serde(default)]
    active_subscription_id: Option<String>,
    #[serde(default)]
    active_user_profile_id: Option<String>,
    #[serde(default)]
    panel_password_hash: Option<String>,
    /// Whether Mihomo core should be auto-started on camofy launch.
    #[serde(default)]
    core_auto_start: bool,
    /// 自动更新远程订阅的定时任务配置
    #[serde(default)]
    subscription_auto_update: Option<ScheduledTaskConfig>,
    /// 自动更新 GeoIP 数据库的定时任务配置
    #[serde(default)]
    geoip_auto_update: Option<ScheduledTaskConfig>,
}

#[tokio::main]
async fn main() {
    let data_root = app::data_root();
    init_tracing(&data_root);

    let state = AppState {
        data_root: data_root.clone(),
        http_client: reqwest::ClientBuilder::new().user_agent("clash-verge/v2.4.3").build().unwrap(),
        auth_tokens: tokio::sync::Mutex::new(Vec::new()),
    };
    if let Err(err) = app::init_data_dirs(&data_root) {
        tracing::error!(
            "failed to initialize data directories at {}: {err}",
            data_root.display()
        );
    } else {
        tracing::info!("data directories ready at {}", data_root.display());
    }

    if app::init_app_state(state).is_err() {
        tracing::error!("failed to set global application state");
        return;
    }

    // 启动后台定时任务调度器（订阅自动更新、GeoIP 数据库自动更新等）
    scheduler::start_scheduler();

    // 根据上次记忆的状态自动启动内核（如果需要）
    core::auto_start_core_if_configured().await;

    let app = build_router();

    let addr = app::server_addr_from_env();
    tracing::info!("starting camofy server at http://{addr}");

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!("failed to bind to {addr}: {err}");
            return;
        }
    };

    if let Err(err) = axum::serve(listener, app).await {
        tracing::error!("server error: {err}");
    }
}

fn init_tracing(data_root: &PathBuf) {
    use std::fs::{self, OpenOptions};
    use std::io::{Result as IoResult, Write};
    use tracing_subscriber::{fmt, EnvFilter};
    use tracing_subscriber::fmt::writer::MakeWriter;

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // Determine <DATA_ROOT>/log/app.log
    let mut log_path = data_root.clone();
    log_path.push("log");
    log_path.push("app.log");

    struct FileWriter {
        path: PathBuf,
    }

    impl Write for FileWriter {
        fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
            if let Some(parent) = self.path.parent() {
                fs::create_dir_all(parent)?;
            }

            // 在写入前尝试进行简单日志轮转，防止单个日志文件过大。
            let _ = crate::logs::rotate_log_file(&self.path);

            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            file.write(buf)
        }

        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    struct FileMakeWriter {
        path: PathBuf,
    }

    impl<'a> MakeWriter<'a> for FileMakeWriter {
        type Writer = FileWriter;

        fn make_writer(&'a self) -> Self::Writer {
            FileWriter {
                path: self.path.clone(),
            }
        }
    }

    fmt()
        .with_env_filter(env_filter)
        .with_writer(FileMakeWriter { path: log_path })
        .init();
}

fn build_router() -> Router {
    let api = Router::new()
        .route("/health", get(health_handler))
        .route("/auth/login", post(auth::auth_login))
        .route("/settings", get(auth::get_settings).put(auth::update_settings))
        .route(
            "/subscriptions",
            get(subscriptions::list_subscriptions).post(subscriptions::create_subscription),
        )
        .route(
            "/subscriptions/:id",
            put(subscriptions::update_subscription).delete(subscriptions::delete_subscription),
        )
        .route(
            "/subscriptions/:id/activate",
            post(subscriptions::activate_subscription),
        )
        .route(
            "/subscriptions/:id/fetch",
            post(subscriptions::fetch_subscription),
        )
        .route(
            "/user-profiles",
            get(user_profiles::list_user_profiles).post(user_profiles::create_user_profile),
        )
        .route(
            "/user-profiles/:id",
            get(user_profiles::get_user_profile)
                .put(user_profiles::update_user_profile)
                .delete(user_profiles::delete_user_profile),
        )
        .route(
            "/user-profiles/:id/activate",
            post(user_profiles::activate_user_profile),
        )
        .route("/core", get(core::get_core_info))
        .route("/core/status", get(core::get_core_status))
        .route("/core/download", post(core::download_core))
        .route("/core/start", post(core::start_core))
        .route("/core/stop", post(core::stop_core))
        .route("/config/merged", get(user_profiles::get_merged_config))
        .route("/logs/app", get(logs::get_app_log))
        .route("/logs/mihomo", get(logs::get_mihomo_log))
        .route("/mihomo/proxies", get(mihomo::get_proxies))
        .route(
            "/mihomo/proxies/:group/select",
            post(mihomo::select_proxy),
        );

    // 为 /api 路由增加认证中间件
    let api = api.layer(middleware::from_fn(auth::api_auth_middleware));

    Router::new()
        .nest("/api", api)
        .route("/", get(static_handler))
        .route("/*path", get(static_handler))
}

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    let candidate = if path.is_empty() { "index.html" } else { path };

    match asset_response(candidate) {
        Some(response) => response,
        None => {
            if !path.contains('.') {
                if let Some(response) = asset_response("index.html") {
                    return response;
                }
            }

            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(header::CONTENT_TYPE, mime::TEXT_PLAIN_UTF_8.as_ref())
                .body(axum::body::Body::from("404 not found"))
                .unwrap()
        }
    }
}

fn asset_response(path: &str) -> Option<Response> {
    let asset = Assets::get(path)?;

    let body = axum::body::Body::from(asset.data.into_owned());

    let mime = mime_guess::from_path(path).first_or_octet_stream();

    Some(
        Response::builder()
            .header(header::CONTENT_TYPE, mime.as_ref())
            .body(body)
            .unwrap(),
    )
}

fn app_config_path(root: &PathBuf) -> PathBuf {
    let mut path = root.clone();
    path.push("config");
    path.push("app.json");
    path
}

fn apply_app_config_defaults(config: &mut AppConfig) {
    // 默认启用订阅与 GeoIP 的自动更新任务，每天凌晨 3 点执行。
    if config.subscription_auto_update.is_none() {
        config.subscription_auto_update = Some(ScheduledTaskConfig {
            cron: "0 3 * * *".to_string(),
            enabled: true,
            last_run_time: None,
            last_run_status: None,
            last_run_message: None,
        });
    }

    if config.geoip_auto_update.is_none() {
        config.geoip_auto_update = Some(ScheduledTaskConfig {
            cron: "0 3 * * *".to_string(),
            enabled: true,
            last_run_time: None,
            last_run_status: None,
            last_run_message: None,
        });
    }
}

pub(crate) fn load_app_config(root: &PathBuf) -> Result<AppConfig, String> {
    use std::fs;
    use std::io::ErrorKind;

    let path = app_config_path(root);
    match fs::read_to_string(&path) {
        Ok(content) => {
            let mut config: AppConfig = serde_json::from_str(&content)
                .map_err(|err| format!("failed to parse app.json at {}: {err}", path.display()))?;
            apply_app_config_defaults(&mut config);
            Ok(config)
        }
        Err(err) => {
            if err.kind() == ErrorKind::NotFound {
                let mut config = AppConfig::default();
                apply_app_config_defaults(&mut config);
                Ok(config)
            } else {
                Err(format!("failed to read app.json at {}: {err}", path.display()))
            }
        }
    }
}

pub(crate) fn save_app_config(root: &PathBuf, config: &AppConfig) -> Result<(), String> {
    use std::fs;

    let path = app_config_path(root);
    let parent = path
        .parent()
        .ok_or_else(|| format!("invalid app.json path: {}", path.display()))?;

    if let Err(err) = fs::create_dir_all(parent) {
        return Err(format!(
            "failed to create config directory at {}: {err}",
            parent.display()
        ));
    }

    let content = serde_json::to_string_pretty(config)
        .map_err(|err| format!("failed to serialize app config: {err}"))?;

    fs::write(&path, content)
        .map_err(|err| format!("failed to write app.json at {}: {err}", path.display()))
}
