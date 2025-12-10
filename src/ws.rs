use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;

use crate::app::{app_state, current_timestamp};
use crate::{AppEvent, CoreOperationState};

pub async fn events_ws(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    let state = app_state();
    let mut rx = state.events_tx.subscribe();

    // 1. 先发送一次当前核心运行状态快照
    let (running, pid) = crate::core::core_running_status(&state.data_root);
    let status_event = AppEvent::CoreStatusChanged {
        running,
        pid,
        timestamp: current_timestamp(),
    };
    if send_event(&mut socket, &status_event).await.is_err() {
        return;
    }

    // 2. 若存在正在进行或最近一次的核心启动/停止操作状态，也发送给新连接
    if let Ok(guard) = state.core_operation.try_lock() {
        if let Some(op_state) = guard.clone() {
            let event = AppEvent::CoreOperationUpdated { state: op_state };
            let _ = send_event(&mut socket, &event).await;
        }
    }

    // 3. 持续将后端广播的 AppEvent 转发给前端 WebSocket 客户端
    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {
                        // 忽略客户端发送的数据，仅作为 keep-alive/心跳
                    }
                    Some(Err(err)) => {
                        tracing::debug!("websocket receive error: {err}");
                        break;
                    }
                }
            }
            event = rx.recv() => {
                match event {
                    Ok(ev) => {
                        if send_event(&mut socket, &ev).await.is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        tracing::debug!("websocket broadcast channel closed: {err}");
                        break;
                    }
                }
            }
        }
    }
}

async fn send_event(socket: &mut WebSocket, event: &AppEvent) -> Result<(), ()> {
    let text = match serde_json::to_string(event) {
        Ok(t) => t,
        Err(err) => {
            tracing::error!("failed to serialize AppEvent for websocket: {err}");
            return Ok(());
        }
    };

    if socket.send(Message::Text(text)).await.is_err() {
        return Err(());
    }

    Ok(())
}

