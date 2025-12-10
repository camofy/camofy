use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::OnceLock;

use tokio::sync::{Mutex, broadcast};

/// Default data root for router environments.
pub const DEFAULT_DATA_ROOT: &str = "/jffs/camofy";
pub const DEFAULT_HOST: &str = "0.0.0.0";
pub const DEFAULT_PORT: u16 = 3000;

#[derive(Clone)]
pub struct AuthSession {
    pub token: String,
    pub expires_at: u64,
}

pub struct AppState {
    pub data_root: PathBuf,
    pub http_client: reqwest::Client,
    pub auth_tokens: Mutex<Vec<AuthSession>>,
    /// 全局应用配置，进程内唯一真相源（SoT）。
    /// 启动时从磁盘加载一次，之后所有读写都通过内存进行，
    /// 磁盘上的 app.json 仅作为持久化备份。
    pub app_config: std::sync::RwLock<crate::AppConfig>,
    /// 全局事件总线，用于向后台任务 / WebSocket 推送应用状态变更。
    pub events_tx: broadcast::Sender<crate::AppEvent>,
}

static APP_STATE: OnceLock<AppState> = OnceLock::new();

pub fn init_app_state(state: AppState) -> Result<(), AppState> {
    APP_STATE.set(state)
}

pub fn app_state() -> &'static AppState {
    APP_STATE
        .get()
        .expect("app state is initialized before the server starts")
}

pub fn data_root() -> PathBuf {
    use std::path::Path;

    let jffs_root = Path::new("/jffs");
    if jffs_root.is_dir() {
        return PathBuf::from(DEFAULT_DATA_ROOT);
    }

    if let Some(home) = std::env::var_os("HOME") {
        let mut base = PathBuf::from(home);
        base.push(".local");
        base.push("share");
        base.push("camofy");
        return base;
    }

    PathBuf::from(DEFAULT_DATA_ROOT)
}

pub fn server_addr_from_env() -> SocketAddr {
    let host = std::env::var("CAMOFY_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string());
    let port = std::env::var("CAMOFY_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);

    format!("{host}:{port}")
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT)))
}

pub fn init_data_dirs(root: &PathBuf) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    for sub in &["config", "core", "log", "tmp"] {
        std::fs::create_dir_all(root.join(sub))?;
    }
    Ok(())
}

pub fn current_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_secs().to_string()
}
