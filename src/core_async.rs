use axum::Json;

use crate::app::app_state;
use crate::{
    ApiResponse, AppEvent, CoreOperationKind, CoreOperationState, CoreOperationStatus,
};

/// 将现有同步的核心启动逻辑封装为“异步任务触发”接口：
/// - 若当前已有启动/停止任务在执行，则返回错误 code。
/// - 否则启动后台任务执行 `core::start_core`，并立即返回。
pub async fn start_core_async() -> Json<ApiResponse<serde_json::Value>> {
    let app = app_state();

    // 若已有核心操作在执行中，则禁止重复提交。
    {
        let guard = app
            .core_operation
            .lock()
            .await;
        if let Some(state) = guard.as_ref() {
            if matches!(state.status, CoreOperationStatus::Running) {
                return Json(ApiResponse {
                    code: "core_operation_in_progress".to_string(),
                    message: "another core operation is in progress".to_string(),
                    data: None,
                });
            }
        }
    }

    // 提前记录一次“启动请求已提交”的运行状态，便于页面刷新后仍能看到“正在启动”。
    {
        let mut guard = app
            .core_operation
            .lock()
            .await;
        let started_at = crate::app::current_timestamp();
        let state = CoreOperationState {
            kind: CoreOperationKind::Start,
            status: CoreOperationStatus::Running,
            message: Some("starting core".to_string()),
            started_at: started_at.clone(),
            finished_at: None,
        };
        *guard = Some(state.clone());
        let event = AppEvent::CoreOperationUpdated { state };
        let _ = app.events_tx.send(event);
    }

    // 后台任务：真正执行核心启动逻辑，并在内部更新 CoreOperationState。
    tokio::spawn(async {
        let Json(resp) = crate::core::start_core().await;
        if resp.code != "ok" {
            tracing::error!(
                "core start task finished with error: code={}, message={}",
                resp.code,
                resp.message
            );
        }
    });

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "start_requested".to_string(),
        data: Some(serde_json::json!({ "operation": "start" })),
    })
}

/// 将同步停止逻辑封装为异步触发接口。
pub async fn stop_core_async() -> Json<ApiResponse<serde_json::Value>> {
    let app = app_state();

    {
        let guard = app
            .core_operation
            .lock()
            .await;
        if let Some(state) = guard.as_ref() {
            if matches!(state.status, CoreOperationStatus::Running) {
                return Json(ApiResponse {
                    code: "core_operation_in_progress".to_string(),
                    message: "another core operation is in progress".to_string(),
                    data: None,
                });
            }
        }
    }

    {
        let mut guard = app
            .core_operation
            .lock()
            .await;
        let started_at = crate::app::current_timestamp();
        let state = CoreOperationState {
            kind: CoreOperationKind::Stop,
            status: CoreOperationStatus::Running,
            message: Some("stopping core".to_string()),
            started_at: started_at.clone(),
            finished_at: None,
        };
        *guard = Some(state.clone());
        let event = AppEvent::CoreOperationUpdated { state };
        let _ = app.events_tx.send(event);
    }

    tokio::spawn(async {
        let Json(resp) = crate::core::stop_core().await;
        if resp.code != "ok" {
            tracing::error!(
                "core stop task finished with error: code={}, message={}",
                resp.code,
                resp.message
            );
        }
    });

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "stop_requested".to_string(),
        data: Some(serde_json::json!({ "operation": "stop" })),
    })
}

