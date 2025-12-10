use crate::app::{app_state, current_timestamp};
use crate::{AppEvent, ConfigChangeReason, CoreReloadResult};

/// 在配置内容发生变更（已成功写入磁盘并生成 `merged.yaml`）后，
/// 根据当前内核运行状态选择性地通知 Mihomo 重新加载配置，并通过全局事件总线上报结果。
///
/// 约定：
/// - 调用方应当在调用本函数前确保：
///   - 相关的 app.json 变更已经写入（通过 `with_app_config_mut` 等）
///   - `merged.yaml` 已根据最新配置生成
/// - 本函数不会修改配置文件，仅负责：
///   - 检测 Mihomo 是否在运行
///   - 若在运行，则通过 Unix Socket 调用控制接口执行配置重载
///   - 将重载结果通过 `AppEvent::ConfigApplied` 广播出去
pub async fn reload_core_if_running(
    reason: ConfigChangeReason,
) -> CoreReloadResult {
    let state = app_state();

    // 检查当前 Mihomo 是否在运行
    let (running, pid) = crate::core::core_running_status(&state.data_root);
    let result = if !running {
        CoreReloadResult::NotRunning
    } else {
        match crate::mihomo::reload_config_with_merged(&state.data_root).await {
            Ok(()) => CoreReloadResult::Reloaded,
            Err(err) => {
                tracing::error!("failed to reload mihomo config: {err}");
                CoreReloadResult::ReloadFailed { message: err }
            }
        }
    };

    // 发送配置应用事件（供后续 WebSocket 等实时通道使用）
    let event = AppEvent::ConfigApplied {
        reason,
        core_reload: result.clone(),
        timestamp: current_timestamp(),
    };

    if let Err(err) = state.events_tx.send(event) {
        // 没有任何订阅者时 send 可能失败，这种情况可以视为正常（仅记录调试日志）
        tracing::debug!("failed to broadcast AppEvent::ConfigApplied: {err}");
    }

    // 额外发送一次内核状态变化事件，便于前端在单一通道上感知状态
    let status_event = AppEvent::CoreStatusChanged {
        running,
        pid,
        timestamp: current_timestamp(),
    };
    if let Err(err) = state.events_tx.send(status_event) {
        tracing::debug!("failed to broadcast AppEvent::CoreStatusChanged: {err}");
    }

    result
}

