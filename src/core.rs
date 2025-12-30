use std::path::{Path, PathBuf};

use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::process::Command as TokioCommand;

use crate::ApiResponse;
use crate::{
    AppEvent, CoreOperationKind, CoreOperationState, CoreOperationStatus,
};
use crate::app::{app_state, current_timestamp};
use crate::{save_app_config, AppConfig};

#[derive(Serialize, Deserialize, Default)]
struct CoreMeta {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    arch: Option<String>,
    #[serde(default)]
    last_download_time: Option<String>,
    #[serde(default)]
    controller_secret: Option<String>,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
}

#[derive(Serialize)]
pub struct CoreInfo {
    version: Option<String>,
    arch: Option<String>,
    last_download_time: Option<String>,
    binary_exists: bool,
    recommended_arch: String,
}

#[derive(Serialize)]
pub struct CoreStatusDto {
    running: bool,
    pid: Option<u32>,
}

#[derive(Deserialize)]
pub struct CoreDownloadRequest {
    #[serde(default)]
    pub url: Option<String>,
}

fn detect_system_arch() -> String {
    #[cfg(target_family = "unix")]
    {
        use std::process::Command;

        if let Ok(output) = Command::new("uname").arg("-m").output() {
            if output.status.success() {
                if let Ok(s) = String::from_utf8(output.stdout) {
                    let arch = s.trim();
                    if !arch.is_empty() {
                        return arch.to_string();
                    }
                }
            }
        }
    }

    std::env::consts::ARCH.to_string()
}

fn map_arch_to_mihomo_arch(arch: &str) -> Option<&'static str> {
    let arch = arch.to_lowercase();
    if arch == "x86_64" || arch == "amd64" {
        Some("linux-amd64")
    } else if arch == "aarch64" || arch == "arm64" {
        Some("linux-arm64")
    } else if arch.starts_with("armv7") {
        Some("linux-armv7")
    } else if arch.starts_with("armv8") {
        Some("linux-armv8")
    } else if arch.starts_with("mipsel") || arch.starts_with("mipsle") {
        Some("linux-mipsle")
    } else if arch.starts_with("mips") {
        Some("linux-mips")
    } else {
        None
    }
}

fn core_dir(root: &PathBuf) -> PathBuf {
    let mut path = root.clone();
    path.push("core");
    path
}

fn core_binary_path(root: &PathBuf) -> PathBuf {
    let mut path = core_dir(root);
    path.push("mihomo");
    path
}

fn core_meta_path(root: &PathBuf) -> PathBuf {
    let mut path = core_dir(root);
    path.push("core.meta.json");
    path
}

fn core_pid_path(root: &PathBuf) -> PathBuf {
    let mut path = core_dir(root);
    path.push("mihomo.pid");
    path
}

pub(crate) fn mihomo_log_path(root: &PathBuf) -> PathBuf {
    let mut path = root.clone();
    path.push("log");
    path.push("mihomo.log");
    path
}

fn load_core_meta(root: &PathBuf) -> CoreMeta {
    use std::fs;
    use std::io::ErrorKind;

    let path = core_meta_path(root);
    match fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<CoreMeta>(&content) {
            Ok(meta) => meta,
            Err(err) => {
                tracing::warn!(
                    "failed to parse core.meta.json at {}: {err}",
                    path.display()
                );
                CoreMeta::default()
            }
        },
        Err(err) => {
            if err.kind() != ErrorKind::NotFound {
                tracing::warn!("failed to read core.meta.json at {}: {err}", path.display());
            }
            CoreMeta::default()
        }
    }
}

fn save_core_meta(root: &PathBuf, meta: &CoreMeta) -> Result<(), String> {
    use std::fs;

    let path = core_meta_path(root);
    let parent = path
        .parent()
        .ok_or_else(|| format!("invalid core.meta.json path: {}", path.display()))?;

    fs::create_dir_all(parent)
        .map_err(|err| format!("failed to create core dir at {}: {err}", parent.display()))?;

    let content = serde_json::to_string_pretty(meta)
        .map_err(|err| format!("failed to serialize core meta: {err}"))?;

    fs::write(&path, content).map_err(|err| {
        format!(
            "failed to write core.meta.json at {}: {err}",
            path.display()
        )
    })
}

pub(crate) fn ensure_controller_secret(root: &PathBuf) -> Result<String, String> {
    let mut meta = load_core_meta(root);

    if let Some(secret) = meta.controller_secret.clone() {
        return Ok(secret);
    }

    let secret = uuid::Uuid::new_v4().to_string();
    meta.controller_secret = Some(secret.clone());

    save_core_meta(root, &meta)?;

    Ok(secret)
}

pub(crate) fn read_core_pid(root: &PathBuf) -> Result<u32, String> {
    use std::fs;
    use std::io::ErrorKind;

    let path = core_pid_path(root);
    match fs::read_to_string(&path) {
        Ok(content) => content
            .trim()
            .parse::<u32>()
            .map_err(|err| format!("invalid pid in {}: {err}", path.display())),
        Err(err) => {
            if err.kind() == ErrorKind::NotFound {
                Err("pid_file_not_found".to_string())
            } else {
                Err(format!(
                    "failed to read pid file at {}: {err}",
                    path.display()
                ))
            }
        }
    }
}

fn write_core_pid(root: &PathBuf, pid: u32) -> Result<(), String> {
    use std::fs;

    let path = core_pid_path(root);
    let parent = path
        .parent()
        .ok_or_else(|| format!("invalid pid path: {}", path.display()))?;

    fs::create_dir_all(parent)
        .map_err(|err| format!("failed to create core dir at {}: {err}", parent.display()))?;

    fs::write(&path, pid.to_string())
        .map_err(|err| format!("failed to write pid file at {}: {err}", path.display()))
}

fn remove_core_pid(root: &PathBuf) {
    use std::fs;
    let path = core_pid_path(root);
    if let Err(err) = fs::remove_file(&path) {
        if err.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("failed to remove pid file {}: {err}", path.display());
        }
    }
}

pub(crate) fn is_process_running(pid: u32) -> bool {
    #[cfg(target_family = "unix")]
    {
        let path = format!("/proc/{pid}");
        std::fs::metadata(path).is_ok()
    }
    #[cfg(not(target_family = "unix"))]
    {
        let _ = pid;
        false
    }
}

#[cfg(target_family = "unix")]
fn ensure_tun_module_loaded() {
    use std::process::Command;

    match Command::new("modprobe").arg("tun").status() {
        Ok(status) if status.success() => {
            tracing::info!("modprobe tun succeeded before core start");
        }
        Ok(status) => {
            tracing::warn!("modprobe tun exited with status {status} before core start");
        }
        Err(err) => {
            tracing::warn!("failed to execute modprobe tun before core start: {err}");
        }
    }
}

#[cfg(not(target_family = "unix"))]
fn ensure_tun_module_loaded() {
}

#[cfg(target_family = "unix")]
fn apply_dns_redirect_rule() {
    use std::process::Command;

    let rule = [
        "-t",
        "nat",
        "-A",
        "PREROUTING",
        "-p",
        "udp",
        "--dport",
        "53",
        "-j",
        "REDIRECT",
        "--to-ports",
        "1053",
    ];

    match Command::new("iptables").args(&rule).status() {
        Ok(status) if status.success() => {
            tracing::info!("iptables dns redirect rule applied successfully");
        }
        Ok(status) => {
            tracing::warn!("failed to apply iptables dns redirect rule, exit status: {status}");
        }
        Err(err) => {
            tracing::warn!("failed to execute iptables to apply dns redirect rule: {err}");
        }
    }
}

#[cfg(not(target_family = "unix"))]
fn apply_dns_redirect_rule() {
}

#[cfg(target_family = "unix")]
fn remove_dns_redirect_rule() {
    use std::process::Command;

    let rule = [
        "-t",
        "nat",
        "-D",
        "PREROUTING",
        "-p",
        "udp",
        "--dport",
        "53",
        "-j",
        "REDIRECT",
        "--to-ports",
        "1053",
    ];

    match Command::new("iptables").args(&rule).status() {
        Ok(status) if status.success() => {
            tracing::info!("iptables dns redirect rule removed successfully");
        }
        Ok(status) => {
            tracing::warn!("failed to remove iptables dns redirect rule, exit status: {status}");
        }
        Err(err) => {
            tracing::warn!("failed to execute iptables to remove dns redirect rule: {err}");
        }
    }
}

#[cfg(not(target_family = "unix"))]
fn remove_dns_redirect_rule() {
}

/// 读取当前 Mihomo 内核的运行状态。
///
/// 返回值：
/// - `(true, Some(pid))`：内核正在运行
/// - `(false, _)`：未运行或 PID 文件不存在 / 损坏（内部会在必要时尝试清理 PID 文件）
pub(crate) fn core_running_status(root: &PathBuf) -> (bool, Option<u32>) {
    match read_core_pid(root) {
        Ok(pid) => {
            if is_process_running(pid) {
                (true, Some(pid))
            } else {
                // PID 文件存在但进程已经不在了，尝试清理
                remove_core_pid(root);
                (false, None)
            }
        }
        Err(reason) => {
            if reason != "pid_file_not_found" {
                tracing::warn!("failed to read core pid: {reason}");
                // 读取 PID 失败时也尝试清理，避免下次重复报错
                remove_core_pid(root);
            }
            (false, None)
        }
    }
}

async fn update_core_operation_state(
    kind: CoreOperationKind,
    status: CoreOperationStatus,
    message: Option<String>,
    progress: Option<f32>,
    finished: bool,
) {
    let app = app_state();
    let mut guard = app.core_operation.lock().await;

    let now = crate::app::current_timestamp();

    let mut state = match guard.take() {
        Some(existing) if existing.kind == kind => {
            let mut updated = existing;
            updated.status = status.clone();
            updated.message = message.clone();
            if let Some(p) = progress {
                updated.progress = Some(p);
            }
            if finished {
                updated.finished_at = Some(now.clone());
            }
            updated
        }
        _ => CoreOperationState {
            kind: kind.clone(),
            status: status.clone(),
            message: message.clone(),
            progress,
            started_at: now.clone(),
            finished_at: if finished { Some(now.clone()) } else { None },
        },
    };

    // 如果是开始新的运行状态，确保 started_at 被设置为当前时间且 finished_at 为空。
    if matches!(status, CoreOperationStatus::Running) && !finished {
        state.started_at = now.clone();
        state.finished_at = None;
    }

    *guard = Some(state.clone());

    let event = AppEvent::CoreOperationUpdated { state };
    if let Err(err) = app.events_tx.send(event) {
        tracing::debug!("failed to broadcast CoreOperationUpdated: {err}");
    }
}
#[cfg(target_family = "unix")]
async fn stop_core_via_ipc() -> Result<(), String> {
    // 使用 clash_verge_service_ipc 提供的 IPC 通道优先停止核心
    if !Path::new(clash_verge_service_ipc::IPC_PATH).exists() {
        return Err("ipc_path_not_found".to_string());
    }

    if let Err(err) = clash_verge_service_ipc::connect().await {
        return Err(format!("ipc_connect_failed: {err}"));
    }

    match clash_verge_service_ipc::stop_clash().await {
        Ok(response) => {
            if response.code > 0 {
                Err(format!("ipc_stop_failed: {}", response.message))
            } else {
                Ok(())
            }
        }
        Err(err) => Err(format!("ipc_stop_error: {err}")),
    }
}

async fn resolve_core_download_url(
    client: &reqwest::Client,
    arch_tag: &str,
) -> Result<(String, String, String), String> {
    let api_url = "https://mirror.camofy.app/repos/MetaCubeX/mihomo/releases/latest";
    tracing::info!("fetching latest core release from {api_url} for arch {arch_tag}");

    let resp = client
        .get(api_url)
        .header("User-Agent", "camofy/0.1.0")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|err| format!("failed to request latest release info: {err}"))?;

    let resp = resp
        .error_for_status()
        .map_err(|err| format!("release info request failed: {err}"))?;

    let release: GithubRelease = resp
        .json()
        .await
        .map_err(|err| format!("failed to parse release json: {err}"))?;

    let tag = release.tag_name;
    let version = tag.trim_start_matches('v').to_string();
    let file_name = format!("mihomo-{arch_tag}-v{version}.gz");
    let url =
        format!("https://mirror.camofy.app/MetaCubeX/mihomo/releases/download/{tag}/{file_name}");

    Ok((url, version, file_name))
}

fn extract_core_binary(data: &[u8], file_name: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;

    let file_name = file_name.to_lowercase();

    if file_name.ends_with(".tar.gz") || file_name.ends_with(".tgz") {
        let gz = flate2::read::GzDecoder::new(data);
        let mut archive = tar::Archive::new(gz);
        for entry in archive
            .entries()
            .map_err(|e| format!("failed to read tar entries: {e}"))?
        {
            let mut entry = entry.map_err(|e| format!("failed to read tar entry: {e}"))?;
            let path = entry
                .path()
                .map_err(|e| format!("failed to get tar entry path: {e}"))?;
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !entry.header().entry_type().is_file() {
                continue;
            }
            if name == "mihomo" || name.contains("mihomo") {
                let mut buf = Vec::new();
                entry
                    .read_to_end(&mut buf)
                    .map_err(|e| format!("failed to read core from archive: {e}"))?;
                return Ok(buf);
            }
        }
        Err("no core binary found in archive".to_string())
    } else if file_name.ends_with(".gz") {
        let mut gz = flate2::read::GzDecoder::new(data);
        let mut buf = Vec::new();
        gz.read_to_end(&mut buf)
            .map_err(|e| format!("failed to decompress core gzip: {e}"))?;
        Ok(buf)
    } else {
        // assume plain binary
        Ok(data.to_vec())
    }
}

pub async fn get_core_info() -> Json<ApiResponse<CoreInfo>> {
    let state = app_state();
    let meta = load_core_meta(&state.data_root);
    let binary_path = core_binary_path(&state.data_root);
    let binary_exists = binary_path.is_file();
    let recommended_arch = detect_system_arch();

    let data = CoreInfo {
        version: meta.version,
        arch: meta.arch,
        last_download_time: meta.last_download_time,
        binary_exists,
        recommended_arch,
    };

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "success".to_string(),
        data: Some(data),
    })
}

pub async fn get_core_status() -> Json<ApiResponse<CoreStatusDto>> {
    let state = app_state();

    let (running, pid) = core_running_status(&state.data_root);
    let data = CoreStatusDto { running, pid };

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "success".to_string(),
        data: Some(data),
    })
}

fn update_core_auto_start_flag(auto_start: bool) {
    let state = app_state();

    let mut guard = state
        .app_config
        .write()
        .expect("app config rwlock poisoned");
    let config: &mut AppConfig = &mut guard;

    config.core_auto_start = auto_start;
    if let Err(err) = save_app_config(&state.data_root, config) {
        tracing::error!("failed to save app config when updating core_auto_start: {err}");
    }
}

pub async fn download_core(Json(body): Json<CoreDownloadRequest>) -> Json<ApiResponse<CoreInfo>> {
    let state = app_state();

    let system_arch = detect_system_arch();
    let arch_tag = match map_arch_to_mihomo_arch(&system_arch) {
        Some(tag) => tag.to_string(),
        None => {
            let msg = format!("unsupported system arch for core download: {system_arch}");
            tracing::error!("{msg}");
            return Json(ApiResponse {
                code: "core_unsupported_arch".to_string(),
                message: msg,
                data: None,
            });
        }
    };

    // 如果请求体中提供了 url，则优先使用用户指定的下载地址；
    // 否则根据架构自动从 GitHub Releases 获取最新稳定版本。
    let (download_url, version_opt, asset_name) = if let Some(url) =
        body.url.as_ref().and_then(|u| {
            let u = u.trim();
            if u.is_empty() {
                None
            } else {
                Some(u.to_string())
            }
        }) {
        let name = url
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("mihomo")
            .to_string();
        (url, None, name)
    } else {
        match resolve_core_download_url(&state.http_client, &arch_tag).await {
            Ok((url, tag_name, name)) => {
                let version = tag_name.trim_start_matches('v').to_string();
                (url, Some(version), name)
            }
            Err(err) => {
                tracing::error!("{err}");
                return Json(ApiResponse {
                    code: "core_resolve_download_url_failed".to_string(),
                    message: err,
                    data: None,
                });
            }
        }
    };

    tracing::info!("downloading core from {download_url}");

    // 记录一次“下载/更新内核”操作的开始状态，便于通过 WebSocket 实时展示进度。
    update_core_operation_state(
        CoreOperationKind::Download,
        CoreOperationStatus::Running,
        Some("downloading core".to_string()),
        Some(0.0),
        false,
    )
    .await;

    let tmp_dir = {
        let mut path = state.data_root.clone();
        path.push("tmp");
        path
    };
    if let Err(err) = std::fs::create_dir_all(&tmp_dir) {
        let msg = format!("failed to create tmp dir {}: {err}", tmp_dir.display());
        tracing::error!("{msg}");
        update_core_operation_state(
            CoreOperationKind::Download,
            CoreOperationStatus::Error,
            Some(msg.clone()),
            None,
            true,
        )
        .await;
        return Json(ApiResponse {
            code: "core_download_failed".to_string(),
            message: msg,
            data: None,
        });
    }

    let tmp_path = tmp_dir.join("mihomo-download.tmp");

    let resp = match state.http_client.get(&download_url).send().await {
        Ok(resp) => resp,
        Err(err) => {
            let msg = format!("failed to send core download request: {err}");
            tracing::error!("{msg}");
            update_core_operation_state(
                CoreOperationKind::Download,
                CoreOperationStatus::Error,
                Some(msg.clone()),
                None,
                true,
            )
            .await;
            return Json(ApiResponse {
                code: "core_download_failed".to_string(),
                message: msg,
                data: None,
            });
        }
    };

    let resp = match resp.error_for_status() {
        Ok(ok) => ok,
        Err(err) => {
            let msg = format!("core download responded with error: {err}");
            tracing::error!("{msg}");
            update_core_operation_state(
                CoreOperationKind::Download,
                CoreOperationStatus::Error,
                Some(msg.clone()),
                None,
                true,
            )
            .await;
            return Json(ApiResponse {
                code: "core_download_failed".to_string(),
                message: msg,
                data: None,
            });
        }
    };

    // 流式读取响应体，以便按字节数更新下载进度。
    let total_len = resp.content_length();
    let mut downloaded: u64 = 0;
    let mut bytes = Vec::new();

    let mut resp = resp;
    while let Some(chunk_result) = resp.chunk().await.transpose() {
        match chunk_result {
            Ok(chunk) => {
                downloaded = downloaded.saturating_add(chunk.len() as u64);
                bytes.extend_from_slice(&chunk);

                if let Some(total) = total_len {
                    if total > 0 {
                        let mut progress = downloaded as f32 / total as f32;
                        if !progress.is_finite() {
                            progress = 0.0;
                        }
                        if progress > 1.0 {
                            progress = 1.0;
                        } else if progress < 0.0 {
                            progress = 0.0;
                        }
                        // 更新下载进度，但不修改 finished_at。
                        update_core_operation_state(
                            CoreOperationKind::Download,
                            CoreOperationStatus::Running,
                            None,
                            Some(progress),
                            false,
                        )
                        .await;
                    }
                }
            }
            Err(err) => {
                let msg = format!("failed to read core download body: {err}");
                tracing::error!("{msg}");
                update_core_operation_state(
                    CoreOperationKind::Download,
                    CoreOperationStatus::Error,
                    Some(msg.clone()),
                    None,
                    true,
                )
                .await;
                return Json(ApiResponse {
                    code: "core_download_failed".to_string(),
                    message: msg,
                    data: None,
                });
            }
        }
    }

    // 解压或直接使用下载内容，取决于文件名后缀
    let core_bytes = match extract_core_binary(&bytes, &asset_name) {
        Ok(b) => b,
        Err(err) => {
            let msg = format!(
                "failed to extract core binary from {}: {err}",
                asset_name.as_str()
            );
            tracing::error!("{msg}");
            update_core_operation_state(
                CoreOperationKind::Download,
                CoreOperationStatus::Error,
                Some(msg.clone()),
                None,
                true,
            )
            .await;
            return Json(ApiResponse {
                code: "core_extract_failed".to_string(),
                message: msg,
                data: None,
            });
        }
    };

    if let Err(err) = std::fs::write(&tmp_path, &core_bytes) {
        let msg = format!(
            "failed to write tmp core file {}: {err}",
            tmp_path.display()
        );
        tracing::error!("{msg}");
        update_core_operation_state(
            CoreOperationKind::Download,
            CoreOperationStatus::Error,
            Some(msg.clone()),
            None,
            true,
        )
        .await;
        return Json(ApiResponse {
            code: "core_install_failed".to_string(),
            message: msg,
            data: None,
        });
    }

    let core_path = core_binary_path(&state.data_root);
    if let Some(parent) = core_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            let msg = format!("failed to create core dir {}: {err}", parent.display());
            tracing::error!("{msg}");
            update_core_operation_state(
                CoreOperationKind::Download,
                CoreOperationStatus::Error,
                Some(msg.clone()),
                None,
                true,
            )
            .await;
            return Json(ApiResponse {
                code: "core_install_failed".to_string(),
                message: msg,
                data: None,
            });
        }
    }

    if let Err(err) = std::fs::rename(&tmp_path, &core_path) {
        let msg = format!("failed to move core file to {}: {err}", core_path.display());
        tracing::error!("{msg}");
        update_core_operation_state(
            CoreOperationKind::Download,
            CoreOperationStatus::Error,
            Some(msg.clone()),
            None,
            true,
        )
        .await;
        return Json(ApiResponse {
            code: "core_install_failed".to_string(),
            message: msg,
            data: None,
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(&core_path) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o755);
            if let Err(err) = std::fs::set_permissions(&core_path, perms) {
                tracing::warn!(
                    "failed to set executable permissions on core binary {}: {err}",
                    core_path.display()
                );
            }
        }
    }

    let mut meta = load_core_meta(&state.data_root);
    meta.arch = Some(arch_tag.clone());
    meta.version = version_opt;
    meta.last_download_time = Some(current_timestamp());

    if let Err(err) = save_core_meta(&state.data_root, &meta) {
        tracing::error!("{err}");
        update_core_operation_state(
            CoreOperationKind::Download,
            CoreOperationStatus::Error,
            Some(err.clone()),
            None,
            true,
        )
        .await;
        return Json(ApiResponse {
            code: "core_meta_save_failed".to_string(),
            message: err,
            data: None,
        });
    }

    tracing::info!("core downloaded and installed at {}", core_path.display());

    update_core_operation_state(
        CoreOperationKind::Download,
        CoreOperationStatus::Success,
        Some("core downloaded and installed".to_string()),
        Some(1.0),
        true,
    )
    .await;

    let info = CoreInfo {
        version: meta.version.clone(),
        arch: meta.arch.clone(),
        last_download_time: meta.last_download_time,
        binary_exists: true,
        recommended_arch: system_arch,
    };

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "downloaded".to_string(),
        data: Some(info),
    })
}

/// 内部核心启动逻辑：完整执行所有检查与进程拉起，返回 JSON 响应。
///
/// 注意：该函数可能耗时较长；对外 API 应通过异步封装（见 `start_core_async`）调用。
pub async fn start_core() -> Json<ApiResponse<serde_json::Value>> {
    let state = app_state();

    // 记录开始启动操作
    update_core_operation_state(
        CoreOperationKind::Start,
        CoreOperationStatus::Running,
        Some("starting core".to_string()),
        None,
        false,
    )
    .await;

    // 启动前确保 geoip.metadb 存在，不存在则先尝试下载。
    let geoip_path = crate::geoip::geoip_target_path(&state.data_root);
    if !geoip_path.is_file() {
        tracing::info!(
            "geoip.metadb not found at {}, trying to download before core start",
            geoip_path.display()
        );
        if let Err(err) = crate::geoip::update_geoip_db().await {
            tracing::error!("failed to download geoip.metadb before core start: {err}");
            // 若下载失败，为避免影响核心启动，这里只记录错误，不直接返回。
        }
    }

    // 检查内核是否已经安装
    let core_path = core_binary_path(&state.data_root);
    if !core_path.is_file() {
        update_core_operation_state(
            CoreOperationKind::Start,
            CoreOperationStatus::Error,
            Some("core binary not found".to_string()),
            None,
            true,
        )
        .await;
        return Json(ApiResponse {
            code: "core_not_installed".to_string(),
            message: "core binary not found".to_string(),
            data: None,
        });
    }

    // 检查是否已在运行
    if let Ok(pid) = read_core_pid(&state.data_root) {
        if is_process_running(pid) {
            update_core_operation_state(
                CoreOperationKind::Start,
                CoreOperationStatus::Error,
                Some(format!("core is already running with pid {}", pid)),
                None,
                true,
            )
            .await;
            return Json(ApiResponse {
                code: "core_already_running".to_string(),
                message: format!("core is already running with pid {}", pid),
                data: None,
            });
        } else {
            // 清理陈旧的 pid 文件
            remove_core_pid(&state.data_root);
        }
    }

    // 启动前确保 merged.yaml 已生成（根据当前订阅和用户配置 + core-defaults.yaml）
    if let Err(err) = crate::user_profiles::generate_merged_config(&state.data_root) {
        tracing::error!("failed to generate merged config before core start: {err}");
        update_core_operation_state(
            CoreOperationKind::Start,
            CoreOperationStatus::Error,
            Some(format!("failed to generate merged config: {err}")),
            None,
            true,
        )
        .await;
        return Json(ApiResponse {
            code: "config_merge_failed".to_string(),
            message: err,
            data: None,
        });
    }

    // 检查配置文件是否存在
    let mut config_dir = state.data_root.clone();
    config_dir.push("config");
    let config_file = config_dir.join("merged.yaml");
    if !config_file.is_file() {
        let msg = format!("config file not found at {}", config_file.display());
        update_core_operation_state(
            CoreOperationKind::Start,
            CoreOperationStatus::Error,
            Some(msg.clone()),
            None,
            true,
        )
        .await;
        return Json(ApiResponse {
            code: "core_config_missing".to_string(),
            message: msg,
            data: None,
        });
    }

    tracing::info!(
        "starting core: binary={} config_dir={} config_file={}",
        core_path.display(),
        config_dir.display(),
        config_file.display()
    );

    // 准备 Mihomo 日志输出路径
    use std::process::Stdio;
    let log_path = mihomo_log_path(&state.data_root);
    // 为 Mihomo 日志创建共享写入状态，用于在磁盘空间不足时统一关闭文件写入。
    let log_state = crate::logs::new_shared_log_write_state();

    // 在真正启动 Mihomo 内核前，尝试加载 tun 内核模块，保证 TUN 模式可用（失败仅记录日志，不中断启动）。
    ensure_tun_module_loaded();

    let mut child = match TokioCommand::new(&core_path)
        .arg("-d")
        .arg(config_dir.as_os_str())
        .arg("-f")
        .arg(config_file.as_os_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false)
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            let msg = format!("failed to spawn core process: {err}");
            tracing::error!("{msg}");
            update_core_operation_state(
                CoreOperationKind::Start,
                CoreOperationStatus::Error,
                Some(msg.clone()),
                None,
                true,
            )
            .await;
            return Json(ApiResponse {
                code: "core_start_failed".to_string(),
                message: msg,
                data: None,
            });
        }
    };

    // 在后台消费 Mihomo 的 stdout/stderr，并通过统一的日志写入封装落盘。
    if let Some(stdout) = child.stdout.take() {
        crate::logs::spawn_log_pipe_task(
            stdout,
            log_path.clone(),
            log_state.clone(),
            "mihomo",
            "stdout",
            true,
        );
    }

    if let Some(stderr) = child.stderr.take() {
        crate::logs::spawn_log_pipe_task(
            stderr,
            log_path.clone(),
            log_state.clone(),
            "mihomo",
            "stderr",
            true,
        );
    }

    let pid = child.id().unwrap_or(0);
    if pid == 0 {
        tracing::warn!("failed to obtain core pid");
    } else if let Err(err) = write_core_pid(&state.data_root, pid) {
        tracing::error!("{err}");
    }

    // 内核进程成功拉起后，再配置 DNS 重定向 iptables 规则。
    apply_dns_redirect_rule();

    // 记忆当前期望的状态为“已启动”，用于下次 camofy 启动时自动拉起内核。
    update_core_auto_start_flag(true);

    // 在后台尝试根据当前配置组合恢复已保存的代理选择，
    // 避免内核重启后用户手动选择的节点丢失。
    tokio::spawn(async {
        // 略微等待内核完成启动流程与配置加载。
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if let Err(err) = crate::mihomo::apply_saved_proxy_selection().await {
            tracing::warn!("failed to apply saved proxy selections after core start: {err}");
        }
    });

    update_core_operation_state(
        CoreOperationKind::Start,
        CoreOperationStatus::Success,
        Some(format!("core started with pid {pid}")),
        None,
        true,
    )
    .await;

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "started".to_string(),
        data: Some(serde_json::json!({ "pid": pid })),
    })
}

pub async fn stop_core() -> Json<ApiResponse<serde_json::Value>> {
    let state = app_state();

    // 停止内核前优先移除 DNS 转发规则，避免仍有新的 DNS 请求被转发到即将关闭的内核。
    remove_dns_redirect_rule();

    // 优先尝试通过 clash_verge_service_ipc 提供的 IPC 通道优雅停止核心
    #[cfg(target_family = "unix")]
    {
        if let Err(err) = stop_core_via_ipc().await {
            tracing::warn!("failed to stop core via IPC: {err}");
        } else {
            tracing::info!("core stopped via IPC");
            remove_core_pid(&state.data_root);
            update_core_auto_start_flag(false);
            update_core_operation_state(
                CoreOperationKind::Stop,
                CoreOperationStatus::Success,
                Some("core stopped via IPC".to_string()),
                None,
                true,
            )
            .await;
            return Json(ApiResponse {
                code: "ok".to_string(),
                message: "stopped".to_string(),
                data: Some(serde_json::json!({ "via": "ipc" })),
            });
        }
    }

    let pid = match read_core_pid(&state.data_root) {
        Ok(pid) => pid,
        Err(reason) => {
            if reason != "pid_file_not_found" {
                tracing::warn!("failed to read core pid when stopping: {reason}");
                remove_core_pid(&state.data_root);
            }
            update_core_operation_state(
                CoreOperationKind::Stop,
                CoreOperationStatus::Error,
                Some("core is not running".to_string()),
                None,
                true,
            )
            .await;
            return Json(ApiResponse {
                code: "core_not_running".to_string(),
                message: "core is not running".to_string(),
                data: None,
            });
        }
    };

    #[cfg(target_family = "unix")]
    {
        use std::process::Command;

        tracing::info!("stopping core with pid {}", pid);

        let status = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();

        match status {
            Ok(status) if status.success() => {
                // 简单地假设终止成功，后续可以根据需要增加等待和 SIGKILL 逻辑
                remove_core_pid(&state.data_root);
                update_core_auto_start_flag(false);
                update_core_operation_state(
                    CoreOperationKind::Stop,
                    CoreOperationStatus::Success,
                    Some("core stopped via signal".to_string()),
                    None,
                    true,
                )
                .await;
                return Json(ApiResponse {
                    code: "ok".to_string(),
                    message: "stopped".to_string(),
                    data: Some(serde_json::json!({ "via": "signal" })),
                });
            }
            Ok(status) => {
                let msg = format!("kill -TERM exited with status {status}");
                tracing::error!("{msg}");
                update_core_operation_state(
                    CoreOperationKind::Stop,
                    CoreOperationStatus::Error,
                    Some(msg.clone()),
                    None,
                    true,
                )
                .await;
                return Json(ApiResponse {
                    code: "core_stop_failed".to_string(),
                    message: msg,
                    data: None,
                });
            }
            Err(err) => {
                let msg = format!("failed to execute kill: {err}");
                tracing::error!("{msg}");
                update_core_operation_state(
                    CoreOperationKind::Stop,
                    CoreOperationStatus::Error,
                    Some(msg.clone()),
                    None,
                    true,
                )
                .await;
                return Json(ApiResponse {
                    code: "core_stop_failed".to_string(),
                    message: msg,
                    data: None,
                });
            }
        }
    }

    #[cfg(not(target_family = "unix"))]
    {
        let _ = pid;
        Json(ApiResponse {
            code: "core_stop_unsupported".to_string(),
            message: "core stop is only supported on unix targets".to_string(),
            data: None,
        })
    }
}

/// 在 camofy 启动时，根据上次记忆的状态自动拉起内核。
pub(crate) async fn auto_start_core_if_configured() {
    let state = app_state();

    let should_auto_start = {
        let guard = state
            .app_config
            .read()
            .expect("app config rwlock poisoned");
        let config: &AppConfig = &guard;
        config.core_auto_start
    };

    if !should_auto_start {
        return;
    }

    // 如果 PID 存在且仍在运行，则无需再次启动。
    match read_core_pid(&state.data_root) {
        Ok(pid) => {
            if is_process_running(pid) {
                tracing::info!("core is already running on startup with pid {}", pid);
                return;
            } else {
                // 清理陈旧的 pid 文件
                remove_core_pid(&state.data_root);
            }
        }
        Err(reason) => {
            if reason != "pid_file_not_found" {
                tracing::warn!("failed to read core pid on startup: {reason}");
                remove_core_pid(&state.data_root);
            }
        }
    }

    // 检查内核是否已经安装
    let core_path = core_binary_path(&state.data_root);
    if !core_path.is_file() {
        tracing::info!(
            "core_auto_start was enabled, but core binary not found at {}",
            core_path.display()
        );
        return;
    }

    // 在系统自启动场景下，为了避免网络尚未就绪导致内核工作异常，
    // 先等待网络连通性基本恢复后再尝试启动 Mihomo。
    if let Err(err) = wait_for_network_ready_before_auto_start().await {
        tracing::warn!(
            "network did not become ready in time before core auto-start: {err}; proceeding anyway"
        );
    }

    tracing::info!("auto-starting core because last state was running (after network ready)");

    let Json(resp) = start_core().await;
    if resp.code != "ok" {
        tracing::error!(
            "failed to auto-start core on camofy launch: code={}, message={}",
            resp.code,
            resp.message
        );
    } else {
        tracing::info!("core auto-started successfully on camofy launch");
    }
}

/// 在自动启动 Mihomo 内核前等待网络“基本就绪”。
///
/// 通过对外部镜像源发起一次简单 HTTP 请求来探测网络连通性：
/// - 请求能成功建立连接并返回任意 HTTP 状态码，即视为网络已就绪；
/// - 若在给定时间内始终无法连通，则返回 Err，但由调用方决定是否继续启动。
async fn wait_for_network_ready_before_auto_start() -> Result<(), String> {
    use tokio::time::{sleep, timeout, Duration, Instant};

    const PROBE_URL: &str = "https://qq.com/";
    const SINGLE_PROBE_TIMEOUT_SECS: u64 = 5;
    const RETRY_INTERVAL_SECS: u64 = 5;
    const MAX_WAIT_SECS: u64 = 300;

    let client = app_state().http_client.clone();

    let max_wait = Duration::from_secs(MAX_WAIT_SECS);
    let probe_timeout = Duration::from_secs(SINGLE_PROBE_TIMEOUT_SECS);
    let retry_interval = Duration::from_secs(RETRY_INTERVAL_SECS);

    tracing::info!(
        "core_auto_start enabled, waiting for network connectivity before starting core (up to {} seconds)",
        MAX_WAIT_SECS
    );

    let start = Instant::now();

    loop {
        // 若已超过最大等待时间，则结束等待。
        if start.elapsed() >= max_wait {
            return Err(format!(
                "network probe to {} did not succeed within {} seconds",
                PROBE_URL, MAX_WAIT_SECS
            ));
        }

        let fut = client.get(PROBE_URL).send();
        match timeout(probe_timeout, fut).await {
            Ok(Ok(_resp)) => {
                tracing::info!(
                    "network connectivity probe to {} succeeded, proceeding with core auto-start",
                    PROBE_URL
                );
                return Ok(());
            }
            Ok(Err(err)) => {
                tracing::warn!(
                    "network probe to {} failed: {}; will retry in {} seconds",
                    PROBE_URL,
                    err,
                    RETRY_INTERVAL_SECS
                );
            }
            Err(_) => {
                tracing::warn!(
                    "network probe to {} timed out after {} seconds; will retry in {} seconds",
                    PROBE_URL,
                    SINGLE_PROBE_TIMEOUT_SECS,
                    RETRY_INTERVAL_SECS
                );
            }
        }

        sleep(retry_interval).await;
    }
}
