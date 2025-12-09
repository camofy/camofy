use axum::Json;

use crate::app::app_state;
use crate::ApiResponse;

pub const LOG_MAX_BYTES: u64 = 1_024 * 1_024; // 1MB
pub const LOG_MAX_ROTATED_FILES: usize = 5;

fn effective_log_max_bytes(path: &std::path::Path) -> u64 {
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
pub fn rotate_log_file(path: &std::path::Path) -> std::io::Result<()> {
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

fn read_log_tail(path: &std::path::Path, max_lines: usize) -> Result<Vec<String>, String> {
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
