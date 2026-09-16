use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

#[derive(Clone)]
pub(super) struct Local {
    pub tx: mpsc::Sender<String>,
    pub status: Arc<Mutex<Value>>,
}
pub(super) fn channel() -> (Local, mpsc::Receiver<String>) {
    let (tx, rx) = mpsc::channel(4);
    (
        Local {
            tx,
            status: Arc::new(Mutex::new(json!({"core_state":"unbound"}))),
        },
        rx,
    )
}
#[derive(Default, Clone, Serialize, Deserialize)]
pub(super) struct Durable {
    #[serde(default)]
    pub stopped: bool,
    #[serde(default)]
    pub command_id: Option<String>,
    #[serde(default)]
    pub command_error: Option<String>,
}
