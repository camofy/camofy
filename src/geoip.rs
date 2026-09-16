use std::path::PathBuf;

use crate::app::app_state;

const GEOIP_URL: &str =
    "https://camofy.app/github/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.metadb";

pub(crate) fn geoip_target_path(root: &PathBuf) -> PathBuf {
    let mut path = root.clone();
    path.push("config");
    path.push("geoip.metadb");
    path
}

fn geoip_tmp_path(root: &PathBuf) -> PathBuf {
    let mut path = root.clone();
    path.push("tmp");
    path.push("geoip.metadb.tmp");
    path
}

/// 读取 merged.yaml 判断当前内核是否使用 dat 模式（`geodata-mode: true`）。
///
/// dat 模式下内核使用 GeoIP.dat / GeoSite.dat，geoip.metadb 不会被加载，
/// camofy 无需下载它；文件缺失或解析失败时按 metadb 模式（默认）处理。
pub(crate) fn merged_uses_dat_mode(root: &PathBuf) -> bool {
    use std::fs;

    let mut path = root.clone();
    path.push("config");
    path.push("merged.yaml");

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    match serde_yaml::from_str::<serde_yaml::Value>(&content) {
        Ok(value) => value
            .get("geodata-mode")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// 下载最新的 geoip.metadb 到 `<DATA_ROOT>/config/geoip.metadb`。
///
/// 当内核运行在 dat 模式（`geodata-mode: true`）时无需 metadb，
/// 返回 `Err("skipped:...")`，由调度器记录为跳过状态。
pub async fn update_geoip_db() -> Result<(), String> {
    use std::fs;

    let state = app_state();
    let root = &state.data_root;

    if merged_uses_dat_mode(root) {
        return Err(
            "skipped:core uses dat mode (geodata-mode: true), geoip.metadb is not needed"
                .to_string(),
        );
    }

    let tmp_path = geoip_tmp_path(root);
    if let Some(parent) = tmp_path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            return Err(format!(
                "failed to create geoip tmp dir at {}: {err}",
                parent.display()
            ));
        }
    }

    let target_path = geoip_target_path(root);
    if let Some(parent) = target_path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            return Err(format!(
                "failed to create geoip target dir at {}: {err}",
                parent.display()
            ));
        }
    }

    tracing::info!("downloading geoip.metadb from {GEOIP_URL}");

    let resp = match state.http_client.get(GEOIP_URL).send().await {
        Ok(resp) => resp,
        Err(err) => {
            return Err(format!("failed to request geoip.metadb: {err}"));
        }
    };

    let resp = match resp.error_for_status() {
        Ok(ok) => ok,
        Err(err) => {
            return Err(format!("geoip.metadb request failed: {err}"));
        }
    };

    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(err) => {
            return Err(format!("failed to read geoip.metadb body: {err}"));
        }
    };

    if let Err(err) = fs::write(&tmp_path, &bytes) {
        return Err(format!(
            "failed to write tmp geoip.metadb at {}: {err}",
            tmp_path.display()
        ));
    }

    if let Err(err) = fs::rename(&tmp_path, &target_path) {
        return Err(format!(
            "failed to move geoip.metadb to {}: {err}",
            target_path.display()
        ));
    }

    tracing::info!(
        "geoip.metadb updated at {} ({} bytes)",
        target_path.display(),
        bytes.len()
    );

    Ok(())
}
