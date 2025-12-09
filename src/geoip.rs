use std::path::PathBuf;

use crate::app::app_state;

const GEOIP_URL: &str =
    "https://mirror.camofy.app/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.metadb";

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

/// 下载最新的 geoip.metadb 到 `<DATA_ROOT>/config/geoip.metadb`。
pub async fn update_geoip_db() -> Result<(), String> {
    use std::fs;

    let state = app_state();
    let root = &state.data_root;

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
