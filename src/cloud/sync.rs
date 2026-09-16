use crate::{
    App, Error, auth,
    store::{self, Resource},
};
use axum::{
    Json,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use sqlx::Row;
use std::time::Duration;
use uuid::Uuid;

#[derive(Clone)]
pub struct Access {
    pub user: Uuid,
    pub bundle: Uuid,
    pub device: Option<Uuid>,
}
pub async fn access(app: &App, token: &str) -> Result<Access, Error> {
    let row = sqlx::query("SELECT user_id,bundle_id,device_id FROM access_tokens WHERE hash=$1")
        .bind(camofy::digest(token))
        .fetch_optional(&app.db)
        .await?
        .ok_or_else(Error::unauthorized)?;
    Ok(Access {
        user: row.get("user_id"),
        bundle: row.get("bundle_id"),
        device: row.get("device_id"),
    })
}
pub async fn current(
    app: &App,
    user: Uuid,
    bundle: Uuid,
    r: &Resource,
) -> Result<(Uuid, Value, Value), Error> {
    // Upgrade old published identities lazily; no mass restart or destructive migration.
    let mut refreshed = None;
    if r.data["system_profile"] != store::system_profile(&app.origin)? {
        let mut tx = app.db.begin().await?;
        store::lock(&mut tx, user).await?;
        store::rebuild(app, &mut tx, user).await?;
        let updated = store::get(app, &mut tx, user, bundle).await?;
        tx.commit().await?;
        if !updated.data["error"].is_null() {
            return Err(Error::new(
                StatusCode::CONFLICT,
                "cannot publish cloud safety profile",
            ));
        }
        refreshed = Some(updated);
    }
    let r = refreshed.as_ref().unwrap_or(r);
    let id = r.data["published_revision"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| {
            Error::new(
                StatusCode::CONFLICT,
                "bundle has no valid published revision",
            )
        })?;
    let row = sqlx::query(
        "SELECT artifacts,selections FROM revisions WHERE id=$1 AND bundle_id=$2 AND user_id=$3",
    )
    .bind(id)
    .bind(bundle)
    .bind(user)
    .fetch_optional(&app.db)
    .await?
    .ok_or_else(Error::not_found)?;
    Ok((
        id,
        app.vault.open(row.get("artifacts"))?,
        row.get("selections"),
    ))
}
async fn snapshot(app: &App, a: &Access) -> Result<Value, Error> {
    let mut conn = app.db.acquire().await?;
    let b = store::get(app, &mut conn, a.user, a.bundle).await?;
    let device = if let Some(id) = a.device {
        Some(store::get(app, &mut conn, a.user, id).await?)
    } else {
        None
    };
    let command = device
        .as_ref()
        .map(|d| d.data["command"].clone())
        .unwrap_or(Value::Null);
    drop(conn);
    let (revision, artifacts, selections) = current(app, a.user, a.bundle, &b).await?;
    let mut result = json!({"revision":revision,"hash":artifacts["router"]["hash"],"selections":selections,"command":command,"identity_name":b.data["name"],"poll_seconds":300});
    result["control"] = json!({"protocol":2,"identity":a.bundle,"binding":device.as_ref().map(crate::control::binding),"selection_version":crate::control::version(&b),"selections":b.data["selections"],"override_version":device.as_ref().map(crate::control::version).unwrap_or(0),"overrides":device.as_ref().map(|d|d.data["selection_overrides"].clone()).unwrap_or(json!({})),"artifact":if artifacts["agent"]["hash"].is_string(){"agent"}else{"router"},"hash":artifacts["agent"]["hash"].as_str().or(artifacts["router"]["hash"].as_str()),"jobs":device.as_ref().map(crate::control::jobs).unwrap_or_default()});
    Ok(result)
}
pub async fn desired(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>, Error> {
    let a = access(&app, auth::bearer(&h).ok_or_else(Error::unauthorized)?).await?;
    Ok(Json(snapshot(&app, &a).await?))
}
pub async fn download(
    State(app): State<App>,
    h: HeaderMap,
    Path((id, format)): Path<(Uuid, String)>,
) -> Result<Response, Error> {
    let a = access(&app, auth::bearer(&h).ok_or_else(Error::unauthorized)?).await?;
    let sealed: Value = sqlx::query_scalar(
        "SELECT artifacts FROM revisions WHERE id=$1 AND user_id=$2 AND bundle_id=$3",
    )
    .bind(id)
    .bind(a.user)
    .bind(a.bundle)
    .fetch_optional(&app.db)
    .await?
    .ok_or_else(Error::not_found)?;
    artifact_response(app.vault.open(sealed)?, &format, &h, id, None)
}
pub async fn subscription(
    State(app): State<App>,
    h: HeaderMap,
    Path((token, format)): Path<(String, String)>,
) -> Result<Response, Error> {
    let a = access(&app, &token).await?;
    auth::rate(&app, format!("sub:{}", camofy::digest(&token)), 120, 60).await?;
    let mut conn = app.db.acquire().await?;
    let b = store::get(&app, &mut conn, a.user, a.bundle).await?;
    drop(conn);
    // Complete any legacy safety migration before taking the read snapshot.
    current(&app, a.user, a.bundle, &b).await?;
    let mut tx = app.db.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let b = store::get(&app, &mut tx, a.user, a.bundle).await?;
    let id = b.data["published_revision"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(Error::not_found)?;
    let sealed: Value = sqlx::query_scalar(
        "SELECT artifacts FROM revisions WHERE id=$1 AND user_id=$2 AND bundle_id=$3",
    )
    .bind(id)
    .bind(a.user)
    .bind(a.bundle)
    .fetch_one(&mut *tx)
    .await?;
    let records = store::list(&app, &mut tx, a.user).await?;
    let usage = crate::usage::published(&app, &mut tx, a.user, &records, &b).await?;
    tx.commit().await?;
    artifact_response(app.vault.open(sealed)?, &format, &h, id, Some(&usage))
}

/// Platform-neutral identity URL. Preserve the existing complete YAML and hash contract;
/// the legacy artifact name is an internal compatibility detail, not a client restriction.
pub async fn identity_subscription(
    app: State<App>,
    h: HeaderMap,
    Path(token): Path<String>,
) -> Result<Response, Error> {
    let mut response = subscription(app, h, Path((token, "router".to_string()))).await?;
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        "attachment; filename=\"camofy.yaml\"".parse().unwrap(),
    );
    Ok(response)
}
fn artifact_response(
    artifacts: Value,
    format: &str,
    h: &HeaderMap,
    revision: Uuid,
    usage: Option<&crate::usage::Summary>,
) -> Result<Response, Error> {
    let a = artifacts.get(format).ok_or_else(Error::not_found)?;
    if let Some(e) = a["error"].as_str() {
        return Err(Error::new(StatusCode::UNPROCESSABLE_ENTITY, e));
    }
    let digest = match usage {
        Some(u) => camofy::digest(serde_json::to_vec(&json!([a["hash"], u.header, u.status]))?),
        None => a["hash"].as_str().unwrap().to_owned(),
    };
    let etag = format!("\"{digest}\"");
    let mut r = if h.get(header::IF_NONE_MATCH).and_then(|h| h.to_str().ok()) == Some(&etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        a["content"].as_str().unwrap().to_string().into_response()
    };
    let headers = r.headers_mut();
    headers.insert(header::ETAG, etag.parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "private, no-cache".parse().unwrap());
    headers.insert(
        header::CONTENT_TYPE,
        if format == "shadowrocket-nodes" {
            "text/plain; charset=utf-8"
        } else {
            "application/yaml; charset=utf-8"
        }
        .parse()
        .unwrap(),
    );
    headers.insert("profile-update-interval", "1".parse().unwrap());
    if let Some(usage) = usage {
        if let Some(value) = &usage.header {
            headers.insert("subscription-userinfo", value.parse().unwrap());
        }
        headers.insert("x-camofy-usage-status", usage.status.parse().unwrap());
        if let Some(at) = usage.updated_at {
            headers.insert("x-camofy-usage-updated-at", at.to_string().parse().unwrap());
        }
    }
    headers.insert("x-camofy-revision", revision.to_string().parse().unwrap());
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!(
            "attachment; filename=\"camofy-{}.{}\"",
            format,
            if format == "shadowrocket-nodes" {
                "txt"
            } else {
                "yaml"
            }
        )
        .parse()
        .unwrap(),
    );
    Ok(r)
}
pub async fn report(
    State(app): State<App>,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> Result<StatusCode, Error> {
    let a = access(&app, auth::bearer(&h).ok_or_else(Error::unauthorized)?).await?;
    let id = a.device.ok_or_else(Error::unauthorized)?;
    if serde_json::to_vec(&body)?.len() > 64 * 1024 {
        return Err(Error::bad("report exceeds 64 KiB"));
    }
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, a.user).await?;
    let mut d = store::get(&app, &mut tx, a.user, id).await?;
    let mut sanitized = json!({"seen_at":crate::now()});
    for k in [
        "revision",
        "status",
        "message",
        "delays",
        "command_id",
        "core_state",
        "command_error",
        "protocol",
    ] {
        if let Some(v) = body.get(k) {
            sanitized[k] = v.clone();
        }
    }
    if !["applied", "failed", "online"].contains(&sanitized["status"].as_str().unwrap_or("")) {
        return Err(Error::bad("invalid report status"));
    }
    if sanitized["message"].as_str().is_some_and(|s| s.len() > 500) {
        return Err(Error::bad("status message too long"));
    }
    if let Some(s) = sanitized["revision"].as_str() {
        let revision = Uuid::parse_str(s).map_err(|_| Error::bad("invalid revision"))?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM revisions WHERE id=$1 AND user_id=$2 AND bundle_id=$3)",
        )
        .bind(revision)
        .bind(a.user)
        .bind(a.bundle)
        .fetch_one(&mut *tx)
        .await?;
        if !exists {
            return Err(Error::bad("revision does not belong to device bundle"));
        }
    }
    if !sanitized["command_id"].is_null() {
        if sanitized["command_id"] != d.data["command"]["id"] {
            return Ok(StatusCode::NO_CONTENT); // Stale measurement from a replaced command.
        }
        if d.data["command"]["type"] == "test_delays" {
            // A background latency result cannot overwrite runtime state.
            for k in ["revision", "status", "message", "core_state"] {
                if let Some(value) = d.data["reported"].get(k) {
                    sanitized[k] = value.clone();
                }
            }
        }
        d.data["command"] = Value::Null;
    } else {
        for k in ["delays", "command_id", "command_error"] {
            if let Some(value) = d.data["reported"].get(k) {
                sanitized[k] = value.clone();
            }
        }
    }
    for k in ["proxy_state", "protocol"] {
        if sanitized.get(k).is_none()
            && let Some(v) = d.data["reported"].get(k)
        {
            sanitized[k] = v.clone();
        }
    }
    d.data["reported"] = sanitized;
    store::put(&app, &mut tx, a.user, &d).await?;
    // Reports do not notify all devices: avoid a feedback loop between sync and telemetry.
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn socket(
    State(app): State<App>,
    h: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, Error> {
    // Browser sessions are same-origin. Agent tokens are sent as the first frame, never URL params.
    let browser = if h.contains_key(header::COOKIE) {
        Some(auth::user(&app, &h, true).await?)
    } else {
        None
    };
    if let Some(origin) = h.get(header::ORIGIN).and_then(|x| x.to_str().ok())
        && !app.accepts_origin(origin)
    {
        return Err(Error::unauthorized());
    }
    Ok(ws
        .max_message_size(4096)
        .on_upgrade(move |ws| run_socket(app, ws, browser, h)))
}
async fn run_socket(app: App, mut ws: WebSocket, browser: Option<Uuid>, headers: HeaderMap) {
    let (user, credential) = if let Some(user) = browser {
        (user, None)
    } else {
        let Ok(Some(Ok(Message::Text(text)))) =
            tokio::time::timeout(Duration::from_secs(10), ws.recv()).await
        else {
            return;
        };
        let Ok(body) = serde_json::from_str::<Value>(&text) else {
            return;
        };
        let Some(token) = body["token"].as_str() else {
            return;
        };
        let Ok(a) = access(&app, token).await else {
            return;
        };
        (a.user, Some(token.to_string()))
    };
    let mut rx = {
        let mut topics = app.topics.lock().await;
        let sender = topics
            .entry(user)
            .or_insert_with(|| tokio::sync::broadcast::channel(16).0);
        if sender.receiver_count() >= 100 {
            return;
        }
        sender.subscribe()
    };
    if ws
        .send(Message::Text(json!({"type":"changed"}).to_string()))
        .await
        .is_err()
    {
        drop(rx);
        let mut topics = app.topics.lock().await;
        if topics.get(&user).is_some_and(|s| s.receiver_count() == 0) {
            topics.remove(&user);
        }
        return;
    }
    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
    let mut last_seen = std::time::Instant::now();
    loop {
        tokio::select! {
            msg=ws.recv()=>match msg {Some(Ok(Message::Pong(_)))|Some(Ok(Message::Text(_)))=>last_seen=std::time::Instant::now(),Some(Ok(Message::Ping(p)))=>{last_seen=std::time::Instant::now();if ws.send(Message::Pong(p)).await.is_err(){break;}},Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,_=>{}},
            _=rx.recv()=>{if ws.send(Message::Text(json!({"type":"changed"}).to_string())).await.is_err(){break;}},
            _=heartbeat.tick()=>{
                if last_seen.elapsed()>Duration::from_secs(90){break;}
                let valid=if let Some(t)=&credential{access(&app,t).await.is_ok()}else{auth::user(&app,&headers,false).await.is_ok()};
                if !valid || ws.send(Message::Ping(Vec::new())).await.is_err(){break;}
            }
        }
    }
    drop(rx);
    let mut topics = app.topics.lock().await;
    if topics.get(&user).is_some_and(|s| s.receiver_count() == 0) {
        topics.remove(&user);
    }
}

pub async fn listen(app: App) {
    tokio::spawn(async move {
        loop {
            let result = async {
                let mut listener = sqlx::postgres::PgListener::connect_with(&app.db).await?;
                listener.listen("camofy_changes").await?;
                // On reconnect clients recheck authoritative state even if NOTIFY was missed.
                for topic in app.topics.lock().await.values() {
                    let _ = topic.send(());
                }
                loop {
                    let msg = listener.recv().await?;
                    if let Ok(user) = Uuid::parse_str(msg.payload())
                        && let Some(topic) = app.topics.lock().await.get(&user)
                    {
                        let _ = topic.send(());
                    }
                }
                #[allow(unreachable_code)]
                Ok::<(), sqlx::Error>(())
            }
            .await;
            if result.is_err() {
                tracing::warn!("notification listener reconnecting");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}
