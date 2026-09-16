use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot};

pub(super) enum Request {
    Core(String),
    Proxy {
        method: String,
        params: Value,
        reply: oneshot::Sender<Result<Value, String>>,
    },
}

#[derive(Clone)]
pub(super) struct Local {
    pub tx: mpsc::Sender<Request>,
    pub status: Arc<Mutex<Value>>,
    pub results: Arc<Mutex<Vec<(String, Value)>>>,
}
pub(super) fn channel() -> (Local, mpsc::Receiver<Request>) {
    let (tx, rx) = mpsc::channel(4);
    (
        Local {
            tx,
            status: Arc::new(Mutex::new(json!({"core_state":"unbound"}))),
            results: Arc::new(Mutex::new(Vec::new())),
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
