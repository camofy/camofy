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
    // Identity is the sole authority. Old per-device overrides are no longer distributed.
    result["control"]["selection_scope"] = json!("identity");
    result["control"]["overrides"] = json!({});
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
    let fallback = filename(&format);
    serve(app, h, token, format, &fallback).await
}
async fn serve(
    app: App,
    h: HeaderMap,
    token: String,
    format: String,
    fallback: &str,
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
    if !crate::client_config::FORMATS.contains(&format.as_str()) {
        return Err(Error::not_found());
    }
    let mut artifacts = app.vault.open(sealed)?;
    let (artifact, report) = crate::client_config::adapt_cached(
        id,
        &artifacts,
        &format,
        h.get(header::USER_AGENT).and_then(|v| v.to_str().ok()),
    )
    .map_err(|e| Error::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    artifacts[&format] = artifact;
    let mut response = artifact_response(artifacts, &format, &h, id, Some(&usage))?;
    response
        .headers_mut()
        .insert(header::VARY, "User-Agent".parse().unwrap());
    response.headers_mut().insert(
        "x-camofy-filtered",
        report.removed.to_string().parse().unwrap(),
    );
    response.headers_mut().insert(
        "x-camofy-compatibility",
        report.client.confidence.parse().unwrap(),
    );
    if let Some(family) = report.client.family {
        response
            .headers_mut()
            .insert("x-camofy-client", family.parse().unwrap());
    }
    // Product/version and aggregate counts only; raw headers, URLs, tokens and
    // node names never enter access diagnostics.
    tracing::info!(
        client = report.client.name.as_deref().unwrap_or("unknown"),
        version = report.client.version.as_deref().unwrap_or("unknown"),
        removed = report.removed,
        retained = report.retained,
        "subscription compatibility applied"
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        disposition(fallback, b.data["name"].as_str().unwrap_or("")),
    );
    Ok(response)
}

/// Platform-neutral identity URL. Preserve the existing complete YAML and hash contract;
/// the legacy artifact name is an internal compatibility detail, not a client restriction.
pub async fn identity_subscription(
    State(app): State<App>,
    h: HeaderMap,
    Path(token): Path<String>,
) -> Result<Response, Error> {
    serve(app, h, token, "router".into(), "camofy.yaml").await
}

fn filename(format: &str) -> String {
    let extension = if format == "shadowrocket-nodes" {
        "txt"
    } else {
        "yaml"
    };
    format!("camofy-{format}.{extension}")
}

/// Clients such as Clash Verge Rev name an imported profile after `filename*`, so it
/// carries the identity name. The header itself must stay ASCII (RFC 5987 encoding).
fn disposition(fallback: &str, name: &str) -> header::HeaderValue {
    const ATTR: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'!')
        .remove(b'#')
        .remove(b'$')
        .remove(b'&')
        .remove(b'+')
        .remove(b'-')
        .remove(b'.')
        .remove(b'^')
        .remove(b'_')
        .remove(b'`')
        .remove(b'|')
        .remove(b'~');
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    let mut value = format!("attachment; filename=\"{fallback}\"");
    if !name.trim().is_empty() {
        value.push_str("; filename*=UTF-8''");
        value.extend(percent_encoding::utf8_percent_encode(name.trim(), ATTR));
    }
    value.parse().unwrap()
}

#[test]
fn disposition_names_profiles_without_non_ascii_headers() {
    assert_eq!(
        disposition("camofy.yaml", "工作 节点;\"x\"\n"),
        "attachment; filename=\"camofy.yaml\"; filename*=UTF-8''%E5%B7%A5%E4%BD%9C%20%E8%8A%82%E7%82%B9%3B%22x%22"
    );
    assert_eq!(
        disposition("camofy-clash.yaml", "Home-1.0"),
        "attachment; filename=\"camofy-clash.yaml\"; filename*=UTF-8''Home-1.0"
    );
    assert_eq!(
        disposition("camofy.yaml", " \t"),
        "attachment; filename=\"camofy.yaml\""
    );
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
        disposition(&filename(format), ""),
    );
    Ok(r)
}
const RUNTIME_REPORT_FIELDS: &[&str] = &[
    "revision",
    "last_successful_revision",
    "attempted_revision",
    "status",
    "message",
    "diagnostic",
    "retry",
    "core_state",
    "core_state_seen_at",
];

/// Normalize old agents before merging telemetry. In the old protocol a failed
/// report's `revision` names the candidate, never evidence of a successful apply.
fn report_state(body: &Value, previous: &Value, now: u64) -> Result<Value, Error> {
    let mut report = json!({"seen_at":now});
    for key in RUNTIME_REPORT_FIELDS.iter().copied().chain([
        "delays",
        "command_id",
        "command_error",
        "protocol",
    ]) {
        if key != "core_state_seen_at"
            && let Some(value) = body.get(key)
        {
            report[key] = value.clone();
        }
    }
    if !["applied", "failed", "online"].contains(&report["status"].as_str().unwrap_or("")) {
        return Err(Error::bad("invalid report status"));
    }
    if !report["message"].is_null()
        && !report["message"]
            .as_str()
            .is_some_and(|value| value.len() <= 500)
    {
        return Err(Error::bad("invalid status message"));
    }
    for key in ["revision", "last_successful_revision", "attempted_revision"] {
        if !report[key].is_null()
            && !report[key]
                .as_str()
                .is_some_and(|value| Uuid::parse_str(value).is_ok())
        {
            return Err(Error::bad("invalid revision"));
        }
    }
    if let Some(diagnostic) = report.get("diagnostic").filter(|value| !value.is_null()) {
        if !diagnostic.is_object()
            || !["stage", "kind", "message"].into_iter().all(|key| {
                diagnostic[key]
                    .as_str()
                    .is_some_and(|value| value.len() <= if key == "message" { 500 } else { 64 })
            })
            || !["exit_code", "signal"].into_iter().all(|key| {
                diagnostic[key].is_null()
                    || diagnostic[key]
                        .as_i64()
                        .is_some_and(|value| i32::try_from(value).is_ok())
            })
            || !diagnostic["output_truncated"].is_boolean()
        {
            return Err(Error::bad("invalid diagnostic"));
        }
        report["diagnostic"] = json!({
            "stage": diagnostic["stage"], "kind": diagnostic["kind"],
            "message": diagnostic["message"], "exit_code": diagnostic["exit_code"],
            "signal": diagnostic["signal"], "output_truncated": diagnostic["output_truncated"],
        });
    }
    if let Some(retry) = report.get("retry").filter(|value| !value.is_null()) {
        if !retry.is_object()
            || !retry["failures"]
                .as_u64()
                .is_some_and(|value| value > 0 && u32::try_from(value).is_ok())
            || !retry["next_retry_at"]
                .as_u64()
                .is_some_and(|value| value <= i64::MAX as u64)
            || !retry["delay_seconds"]
                .as_u64()
                .is_some_and(|value| value > 0 && value <= 86_400)
        {
            return Err(Error::bad("invalid retry state"));
        }
        report["retry"] = json!({"failures":retry["failures"], "next_retry_at":retry["next_retry_at"], "delay_seconds":retry["delay_seconds"]});
    }

    let last_success = if body.get("last_successful_revision").is_some() {
        body["last_successful_revision"].clone()
    } else if (body["status"] == "applied" || body["status"] == "online")
        && body["revision"].is_string()
    {
        body["revision"].clone()
    } else if previous.get("last_successful_revision").is_some() {
        previous["last_successful_revision"].clone()
    } else if previous["status"] == "applied" || previous["status"] == "online" {
        previous["revision"].clone()
    } else {
        Value::Null
    };
    report["attempted_revision"] = body
        .get("attempted_revision")
        .cloned()
        .unwrap_or_else(|| body["revision"].clone());
    report["revision"] = last_success.clone();
    report["last_successful_revision"] = last_success;

    if report["core_state"].is_null() {
        if let Some(core) = previous.get("core_state") {
            report["core_state"] = core.clone();
            report["core_state_seen_at"] = previous
                .get("core_state_seen_at")
                .unwrap_or(&previous["seen_at"])
                .clone();
        }
    } else {
        if !["running", "stopping", "stopped", "unavailable", "unbound"]
            .contains(&report["core_state"].as_str().unwrap_or(""))
        {
            return Err(Error::bad("invalid core state"));
        }
        report["core_state_seen_at"] = json!(now);
    }
    Ok(report)
}

#[cfg(test)]
mod report_tests {
    use super::*;

    #[test]
    fn legacy_failure_preserves_success_and_timestamped_core_state() {
        let successful = Uuid::new_v4().to_string();
        let candidate = Uuid::new_v4().to_string();
        let previous = json!({"status":"applied", "revision":successful, "core_state":"running", "seen_at":100});
        let failed = report_state(
            &json!({"status":"failed", "revision":candidate}),
            &previous,
            200,
        )
        .unwrap();
        assert_eq!(failed["revision"], successful);
        assert_eq!(failed["last_successful_revision"], successful);
        assert_eq!(failed["attempted_revision"], candidate);
        assert_eq!(failed["core_state"], "running");
        assert_eq!(failed["core_state_seen_at"], 100);
        assert_eq!(failed["seen_at"], 200);
        let repeated = report_state(
            &json!({"status":"failed", "revision":candidate}),
            &failed,
            300,
        )
        .unwrap();
        assert_eq!(repeated["last_successful_revision"], successful);
        assert_eq!(repeated["core_state_seen_at"], 100);

        let unknown = report_state(
            &json!({"status":"failed", "revision":candidate}),
            &Value::Null,
            200,
        )
        .unwrap();
        assert!(unknown["last_successful_revision"].is_null());
        assert!(unknown["revision"].is_null());
        let still_unknown = report_state(
            &json!({"status":"failed", "revision":candidate}),
            &json!({"status":"failed", "revision":candidate}),
            300,
        )
        .unwrap();
        assert!(still_unknown["last_successful_revision"].is_null());
    }

    #[test]
    fn modern_failure_keeps_bounded_diagnostics_and_clears_on_recovery() {
        let candidate = Uuid::new_v4().to_string();
        let failed = report_state(&json!({
            "status":"failed", "revision":null, "last_successful_revision":null, "attempted_revision":candidate,
            "core_state":"unavailable", "core_state_seen_at":1,
            "diagnostic":{"stage":"validation", "kind":"out_of_memory", "message":"Mihomo ran out of memory", "exit_code":2, "signal":null, "output_truncated":true, "logs":"private output"},
            "retry":{"failures":3,"next_retry_at":320,"delay_seconds":120,"extra":"private output"},
            "logs":"private output"
        }), &Value::Null, 200).unwrap();
        assert_eq!(failed["core_state_seen_at"], 200);
        assert_eq!(failed["diagnostic"]["kind"], "out_of_memory");
        assert_eq!(failed["retry"]["failures"], 3);
        assert!(!failed.to_string().contains("private output"));
        let recovered = report_state(&json!({"status":"applied", "revision":candidate, "last_successful_revision":candidate,"attempted_revision":candidate,"core_state":"running","diagnostic":null,"retry":null}), &failed, 400).unwrap();
        assert_eq!(recovered["last_successful_revision"], candidate);
        assert_eq!(recovered["core_state_seen_at"], 400);
        assert!(recovered["diagnostic"].is_null());
        assert!(recovered["retry"].is_null());

        let mut invalid = failed;
        invalid["retry"]["failures"] = json!(-1);
        assert!(report_state(&invalid, &Value::Null, 300).is_err());
        invalid["retry"] = Value::Null;
        invalid["diagnostic"]["message"] = json!("x".repeat(501));
        assert!(report_state(&invalid, &Value::Null, 300).is_err());
        invalid["diagnostic"] = Value::Null;
        invalid["attempted_revision"] = json!("not-a-revision");
        assert!(report_state(&invalid, &Value::Null, 300).is_err());
    }

    #[test]
    fn legacy_control_report_without_revision_retains_known_success() {
        let revision = Uuid::new_v4().to_string();
        let report = report_state(
            &json!({"status":"online","revision":null,"core_state":"stopped"}),
            &json!({"status":"applied","revision":revision,"core_state":"running","seen_at":100}),
            200,
        )
        .unwrap();
        assert_eq!(report["last_successful_revision"], revision);
        assert_eq!(report["core_state"], "stopped");
        assert_eq!(report["core_state_seen_at"], 200);
    }
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
    let mut sanitized = report_state(&body, &d.data["reported"], crate::now())?;
    let mut checked = std::collections::HashSet::new();
    for key in ["revision", "last_successful_revision", "attempted_revision"] {
        let Some(s) = sanitized[key].as_str() else {
            continue;
        };
        if !checked.insert(s.to_owned()) {
            continue;
        }
        let revision = Uuid::parse_str(s).map_err(|_| Error::bad("invalid revision"))?;
        let owner_matches: Option<bool> =
            sqlx::query_scalar("SELECT user_id=$2 AND bundle_id=$3 FROM revisions WHERE id=$1")
                .bind(revision)
                .bind(a.user)
                .bind(a.bundle)
                .fetch_optional(&mut *tx)
                .await?;
        // Revision artifacts are pruned independently of a device's last good
        // configuration. An absent historical success is device-reported metadata,
        // not permission to read an artifact. Existing foreign revisions remain
        // forbidden, and a new candidate must still belong to this identity.
        if owner_matches == Some(false) || (owner_matches.is_none() && key == "attempted_revision")
        {
            return Err(Error::bad("revision does not belong to device bundle"));
        }
    }
    if !sanitized["command_id"].is_null() {
        if sanitized["command_id"] != d.data["command"]["id"] {
            tracing::info!(device_id = %id, "stale device command report ignored");
            return Ok(StatusCode::NO_CONTENT); // Stale measurement from a replaced command.
        }
        if d.data["command"]["type"] == "test_delays" && d.data["reported"]["status"].is_string() {
            // A background latency result cannot overwrite runtime state.
            let previous = report_state(
                &d.data["reported"],
                &Value::Null,
                d.data["reported"]["seen_at"]
                    .as_u64()
                    .unwrap_or(crate::now()),
            )?;
            for &key in RUNTIME_REPORT_FIELDS {
                if let Some(value) = previous.get(key) {
                    sanitized[key] = value.clone();
                } else {
                    sanitized.as_object_mut().unwrap().remove(key);
                }
            }
            if let Some(at) = d.data["reported"].get("core_state_seen_at") {
                sanitized["core_state_seen_at"] = at.clone();
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
    if d.data["reported"]["status"] == "failed" {
        tracing::warn!(device_id = %id, bundle_id = %a.bundle,
            revision = ?d.data["reported"]["attempted_revision"].as_str(),
            failure_kind = ?d.data["reported"]["diagnostic"]["kind"].as_str(),
            "device reported configuration failure");
    }
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
            if let Err(error) = result {
                tracing::warn!(database_code = ?error.as_database_error().and_then(|e| e.code()),
                    "notification listener reconnecting");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}
