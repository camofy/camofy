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

use crate::app::app_state;

mod app;
mod auth;
mod config_manager;
mod core;
mod core_async;
mod ws;
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

/// 记录某个代理组当前选择的节点。
#[derive(Serialize, Deserialize, Clone, Default)]
pub(crate) struct ProxySelectionRecord {
    pub group: String,
    pub node: String,
}

/// 针对一组配置组合（订阅 + 用户配置）的代理选择快照。
#[derive(Serialize, Deserialize, Clone, Default)]
pub(crate) struct ProxySelectionSet {
    /// 对应 AppConfig.active_subscription_id
    #[serde(default)]
    pub subscription_id: Option<String>,
    /// 对应 AppConfig.active_user_profile_id
    #[serde(default)]
    pub user_profile_id: Option<String>,
    #[serde(default)]
    pub selections: Vec<ProxySelectionRecord>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub(crate) struct AppConfig {
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
    /// 针对不同订阅 + 用户配置组合保存的代理选择快照。
    #[serde(default)]
    proxy_selections: Vec<ProxySelectionSet>,
}

/// 对应“是什么导致了配置需要被应用 / 重新加载”的高层原因，
/// 便于后续在日志或 WebSocket 事件中做区分。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ConfigChangeReason {
    SubscriptionFetched,
    ActiveSubscriptionChanged,
    SubscriptionDeleted,
    UserProfileUpdated,
    ActiveUserProfileChanged,
    UserProfileDeleted,
    SettingsUpdated,
    Other,
}

/// 描述一次“尝试向 Mihomo 应用配置”的结果。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CoreReloadResult {
    NotRunning,
    Reloaded,
    ReloadFailed { message: String },
    Skipped { reason: String },
}

/// 后台向前端广播的应用级事件模型。
/// 后续可以通过 WebSocket 订阅这些事件，实现实时状态更新。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppEvent {
    ConfigApplied {
        reason: ConfigChangeReason,
        core_reload: CoreReloadResult,
        timestamp: String,
    },
    CoreStatusChanged {
        running: bool,
        pid: Option<u32>,
        timestamp: String,
    },
    CoreOperationUpdated {
        state: CoreOperationState,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoreOperationKind {
    Start,
    Stop,
    Download,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoreOperationStatus {
    Pending,
    Running,
    Success,
    Error,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoreOperationState {
    pub kind: CoreOperationKind,
    pub status: CoreOperationStatus,
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[tokio::main]
async fn main() {
    let data_root = app::data_root();
    init_tracing(&data_root);

    // 启动时从磁盘加载 app.json，失败则直接退出进程。
    let app_config = match load_app_config(&data_root) {
        Ok(cfg) => cfg,
        Err(err) => {
            tracing::error!("{err}");
            std::process::exit(1);
        }
    };

    // 全局事件广播通道（后续可用于 WebSocket 实时推送）
    let (events_tx, _events_rx) = tokio::sync::broadcast::channel(128);

    let state = AppState {
        data_root: data_root.clone(),
        http_client: reqwest::ClientBuilder::new()
            .user_agent("clash-verge/v2.4.3")
            // 为所有 HTTP 请求设置一个上限，防止下载或远程请求无限挂起。
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap(),
        auth_tokens: tokio::sync::Mutex::new(Vec::new()),
        app_config: std::sync::RwLock::new(app_config),
        events_tx,
        core_operation: tokio::sync::Mutex::new(None),
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

    // 根据上次记忆的状态自动启动内核（如果需要）。
    // 放到后台任务中执行，内部会在尝试启动前等待网络连通性恢复，
    // 避免在路由器刚开机、网络尚未就绪时阻塞 Web 服务启动。
    tokio::spawn(async {
        core::auto_start_core_if_configured().await;
    });

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
        .route("/core/start", post(core_async::start_core_async))
        .route("/core/stop", post(core_async::stop_core_async))
        .route("/core/restart", post(core_async::restart_core_async))
        .route("/config/merged", get(user_profiles::get_merged_config))
        .route("/logs/app", get(logs::get_app_log))
        .route("/logs/mihomo", get(logs::get_mihomo_log))
        .route("/mihomo/proxies", get(mihomo::get_proxies))
        .route(
            "/mihomo/proxies/:group/select",
            post(mihomo::select_proxy),
        )
        .route("/events/ws", get(ws::events_ws));

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

/// 读取当前全局 AppConfig 的只读快照。
pub(crate) fn get_app_config_snapshot() -> AppConfig {
    let state = app_state();
    let guard = state.app_config.read().expect("app config rwlock poisoned");
    guard.clone()
}

/// 对全局 AppConfig 进行一次原子更新，并将结果持久化到磁盘。
///
/// - `f` 在持有写锁的情况下被调用，可以对配置做任意修改。
/// - 修改完成后会将配置写回 `<DATA_ROOT>/config/app.json`。
/// - 磁盘写入失败时，内存中的修改仍然保留（以内存为准），返回 Err。
pub(crate) fn with_app_config_mut<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce(&mut AppConfig) -> R,
{
    let state = app_state();
    let mut guard = state
        .app_config
        .write()
        .expect("app config rwlock poisoned");

    let result = f(&mut guard);

    // 尝试将修改持久化到磁盘。失败时返回错误，但不回滚内存中的修改。
    save_app_config(&state.data_root, &guard)?;

    Ok(result)
}

/// 在当前活跃配置组合下，更新指定代理组的已选节点并持久化到 app.json。
pub(crate) fn update_proxy_selection_for_current_profile(
    group: &str,
    node: &str,
) -> Result<(), String> {
    with_app_config_mut(|config: &mut AppConfig| {
        let sub_id = config.active_subscription_id.clone();
        let user_id = config.active_user_profile_id.clone();

        // 先尝试找到当前组合对应的快照，没有则新建。
        let set = if let Some(existing) = config
            .proxy_selections
            .iter_mut()
            .find(|s| s.subscription_id == sub_id && s.user_profile_id == user_id)
        {
            existing
        } else {
            config.proxy_selections.push(ProxySelectionSet {
                subscription_id: sub_id,
                user_profile_id: user_id,
                selections: Vec::new(),
            });
            config
                .proxy_selections
                .last_mut()
                .expect("proxy_selections just pushed must exist");
            config.proxy_selections.last_mut().unwrap()
        };

        if let Some(rec) = set
            .selections
            .iter_mut()
            .find(|r| r.group == group)
        {
            rec.node = node.to_string();
        } else {
            set.selections.push(ProxySelectionRecord {
                group: group.to_string(),
                node: node.to_string(),
            });
        }
    })?;

    Ok(())
}

/// 读取当前活跃配置组合下保存的代理选择快照。
pub(crate) fn get_proxy_selections_for_active_profile(
) -> Option<Vec<ProxySelectionRecord>> {
    let cfg = get_app_config_snapshot();
    let sub_id = cfg.active_subscription_id.clone();
    let user_id = cfg.active_user_profile_id.clone();

    cfg.proxy_selections
        .into_iter()
        .find(|s| s.subscription_id == sub_id && s.user_profile_id == user_id)
        .map(|s| s.selections)
}
