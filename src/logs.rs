use axum::Json;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::app::{app_state, current_timestamp};
use crate::{ApiResponse, AppEvent};

pub const LOG_MAX_BYTES: u64 = 1_024 * 1_024; // 1MB
pub const LOG_MAX_ROTATED_FILES: usize = 5;
// 当剩余磁盘空间低于该值时，尝试清理旧日志；仍不足则关闭该日志的文件写入。
pub const LOG_MIN_FREE_SPACE_BYTES: u64 = LOG_MAX_BYTES;
// 为避免每次写入都触发磁盘空间检查，这里设置一个简单的字节间隔。
pub const LOG_DISK_CHECK_INTERVAL_BYTES: u64 = 64 * 1_024;

#[derive(Default)]
pub struct LogWriteState {
    pub logging_disabled: bool,
    pub bytes_since_last_disk_check: u64,
    pub warning_emitted: bool,
}

pub type SharedLogWriteState = Arc<Mutex<LogWriteState>>;

pub fn new_shared_log_write_state() -> SharedLogWriteState {
    Arc::new(Mutex::new(LogWriteState::default()))
}

fn effective_log_max_bytes(path: &Path) -> u64 {
    // 基础上限
    let mut max_bytes = LOG_MAX_BYTES;

    // 尝试根据磁盘可用空间动态收紧上限：不超过剩余可用空间的 80%
    if let Some(parent) = path.parent() {
        if let Ok(free) = fs2::available_space(parent) {
            let cap = free.saturating_mul(80).saturating_div(100);
            if cap > 0 {
                max_bytes = max_bytes.min(cap);
            }
        }
    }

    max_bytes
}

/// 简单的日志轮转：当文件大小超过 LOG_MAX_BYTES 时，
/// 将当前文件依次按 .1 ~ .LOG_MAX_ROTATED_FILES 滚动，并清理最旧的文件。
pub fn rotate_log_file(path: &Path) -> std::io::Result<()> {
    use std::fs;
    use std::io::ErrorKind;

    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };

    let max_bytes = effective_log_max_bytes(path);
    if meta.len() < max_bytes {
        return Ok(());
    }

    let base = path.to_string_lossy().to_string();

    for idx in (1..=LOG_MAX_ROTATED_FILES).rev() {
        let src = if idx == 1 {
            std::path::PathBuf::from(&base)
        } else {
            std::path::PathBuf::from(format!("{base}.{}", idx - 1))
        };

        if !src.exists() {
            continue;
        }

        let dst = std::path::PathBuf::from(format!("{base}.{idx}"));
        let _ = fs::remove_file(&dst);
        let _ = fs::rename(&src, &dst);
    }

    Ok(())
}

/// 在磁盘空间不足时，尝试清理当前日志对应的所有轮转文件（不删除主文件）。
fn cleanup_rotated_logs(path: &Path) {
    use std::fs;

    let base = path.to_string_lossy().to_string();

    for idx in 1..=LOG_MAX_ROTATED_FILES {
        let rotated = PathBuf::from(format!("{base}.{idx}"));
        if rotated.exists() {
            let _ = fs::remove_file(&rotated);
        }
    }
}

/// 包装后的文件写入逻辑：
/// - 定期检查剩余磁盘空间，低于 LOG_MIN_FREE_SPACE_BYTES 时尝试清理旧日志；仍不足则关闭文件写入； 
/// - 每次写入前尝试轮转日志，以控制单个文件大小；
/// - 当写入被关闭后，继续“吞掉”内容但不再落盘，避免阻塞上游。
pub fn write_log_with_rotation_and_space_guard(
    path: &Path,
    state: &SharedLogWriteState,
    buf: &[u8],
    log_name: &str,
) -> std::io::Result<usize> {
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    {
        let mut inner = state.lock().unwrap();
        if inner.logging_disabled {
            return Ok(buf.len());
        }

        inner.bytes_since_last_disk_check = inner
            .bytes_since_last_disk_check
            .saturating_add(buf.len() as u64);

        if inner.bytes_since_last_disk_check >= LOG_DISK_CHECK_INTERVAL_BYTES {
            inner.bytes_since_last_disk_check = 0;
            // 在锁外查空间与做清理，避免持锁时间过长。
            drop(inner);

            if let Some(parent) = path.parent() {
                if let Ok(free) = fs2::available_space(parent) {
                    if free < LOG_MIN_FREE_SPACE_BYTES {
                        // 先尝试清理旧日志文件。
                        cleanup_rotated_logs(path);

                        if let Ok(free_after) = fs2::available_space(parent) {
                            if free_after < LOG_MIN_FREE_SPACE_BYTES {
                                let mut inner = state.lock().unwrap();
                                inner.logging_disabled = true;
                                if !inner.warning_emitted {
                                    inner.warning_emitted = true;
                                    let path_display = path.display().to_string();
                                    drop(inner);
                                    tracing::warn!(
                                        "disabling {} log file writing at {}: free space below {} bytes",
                                        log_name,
                                        path_display,
                                        LOG_MIN_FREE_SPACE_BYTES
                                    );
                                }
                                return Ok(buf.len());
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // 写入前尝试进行日志轮转，控制单个文件大小。
    let _ = rotate_log_file(path);

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    file.write(buf)
}

/// 从异步读取端持续读取数据并写入日志文件。
/// 用于将子进程的 stdout/stderr 管道内容转发到指定日志文件。
pub fn spawn_log_pipe_task<R>(
    mut reader: R,
    path: PathBuf,
    state: SharedLogWriteState,
    log_name: &'static str,
    direction: &'static str,
    broadcast: bool,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;

        let tx = if broadcast {
            let app = app_state();
            Some(app.events_tx.clone())
        } else {
            None
        };

        let mut buf = [0u8; 4096];

        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if let Err(err) = crate::logs::write_log_with_rotation_and_space_guard(
                        &path,
                        &state,
                        &buf[..n],
                        log_name,
                    ) {
                        tracing::warn!(
                            "failed to write {} {} log: {err}",
                            log_name,
                            direction
                        );
                        break;
                    }

                    if let Some(tx) = &tx {
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                        let event = AppEvent::MihomoLogChunk {
                            stream: direction.to_string(),
                            chunk,
                            timestamp: current_timestamp(),
                        };
                        if let Err(err) = tx.send(event) {
                            tracing::debug!(
                                "failed to broadcast mihomo log chunk via websocket: {err}"
                            );
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        "failed to read {} {}: {err}",
                        log_name,
                        direction
                    );
                    break;
                }
            }
        }
    });
}

fn read_log_tail(path: &Path, max_lines: usize) -> Result<Vec<String>, String> {
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    let file = match File::open(path) {
        Ok(f) => f,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                return Err("log_not_found".to_string());
            }
            return Err(format!("failed to open log file {}: {err}", path.display()));
        }
    };

    let reader = BufReader::new(file);
    let mut ring: std::collections::VecDeque<String> =
        std::collections::VecDeque::with_capacity(max_lines);

    for line in reader.lines() {
        let line =
            line.map_err(|err| format!("failed to read log file {}: {err}", path.display()))?;
        if ring.len() == max_lines {
            ring.pop_front();
        }
        ring.push_back(line);
    }

    Ok(ring.into_iter().collect())
}

pub async fn get_app_log() -> Json<ApiResponse<serde_json::Value>> {
    let state = app_state();
    let mut path = state.data_root.clone();
    path.push("log");
    path.push("app.log");

    match read_log_tail(&path, 200) {
        Ok(lines) => Json(ApiResponse {
            code: "ok".to_string(),
            message: "success".to_string(),
            data: Some(serde_json::json!({ "lines": lines })),
        }),
        Err(err) => {
            if err == "log_not_found" {
                Json(ApiResponse {
                    code: "log_not_found".to_string(),
                    message: "app log not found".to_string(),
                    data: None,
                })
            } else {
                tracing::error!("failed to read app log: {err}");
                Json(ApiResponse {
                    code: "log_read_failed".to_string(),
                    message: err,
                    data: None,
                })
            }
        }
    }
}

pub async fn get_mihomo_log() -> Json<ApiResponse<serde_json::Value>> {
    let state = app_state();
    let path = crate::core::mihomo_log_path(&state.data_root);

    match read_log_tail(&path, 200) {
        Ok(lines) => Json(ApiResponse {
            code: "ok".to_string(),
            message: "success".to_string(),
            data: Some(serde_json::json!({ "lines": lines })),
        }),
        Err(err) => {
            if err == "log_not_found" {
                Json(ApiResponse {
                    code: "log_not_found".to_string(),
                    message: "mihomo log not found".to_string(),
                    data: None,
                })
            } else {
                tracing::error!("failed to read mihomo log: {err}");
                Json(ApiResponse {
                    code: "log_read_failed".to_string(),
                    message: err,
                    data: None,
                })
            }
        }
    }
}
