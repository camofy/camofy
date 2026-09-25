//! Durable desired selections and bounded, tenant-scoped RPC mailboxes.
use crate::{App, Error, auth, store, sync};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use camofy::protocol::{Group, Job, RpcRequest, Selections};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub fn binding(d: &store::Resource) -> String {
    d.data["binding_generation"]
        .as_str()
        .or(d.data["bundle_id"].as_str())
        .unwrap_or("")
        .to_owned()
}
pub fn version(r: &store::Resource) -> u64 {
    r.data["selection_version"].as_u64().unwrap_or(0)
}
pub fn events(r: &store::Resource) -> Vec<Value> {
    r.data["selection_events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}
pub(super) fn record_selection(r: &mut store::Resource, previous: &Value, source: &str) {
    let field = if r.kind == "bundle" {
        "selections"
    } else {
        "selection_overrides"
    };
    let current = &r.data[field];
    let keys: std::collections::BTreeSet<_> = previous
        .as_object()
        .into_iter()
        .flat_map(|o| o.keys())
        .chain(current.as_object().into_iter().flat_map(|o| o.keys()))
        .cloned()
        .collect();
    let mut history = events(r);
    for group in keys {
        if previous[&group] == current[&group] {
            continue;
        }
        for old in &mut history {
            if old["group"] == group && old["status"] == "pending" {
                old["status"] = json!("superseded");
            }
        }
        history.push(
            json!({"id":Uuid::new_v4(),"created_at":crate::now(),"group":group,
            "from":previous[&group],"to":current[&group],"source":source,"version":version(r),
            "status":if r.kind=="bundle" {"saved"} else {"pending"}}),
        );
    }
    let start = history.len().saturating_sub(80);
    r.data["selection_events"] = json!(&history[start..]);
}
fn settle_events(d: &mut store::Resource) {
    let state = &d.data["reported"]["proxy_state"];
    let mut history = events(d);
    for event in &mut history {
        if event["status"] != "pending"
            || event["version"].as_u64().unwrap_or(u64::MAX)
                > state["selection_version"].as_u64().unwrap_or(0)
        {
            continue;
        }
        let group = event["group"].as_str().unwrap_or("");
        if event["scope"] != "identity" {
            event["status"] = json!("superseded");
            continue;
        }
        if event["version"].as_u64() < state["selection_version"].as_u64()
            && !event["to"].is_null()
            && event["to"] != state["desired"][group]
        {
            event["status"] = json!("superseded");
            continue;
        }
        if state["pending_local"] == true {
            continue;
        }
        if !state["errors"][group].is_null() {
            event["detail"] = state["errors"][group].clone();
            continue;
        }
        let desired = &state["desired"][group];
        let expected = if event["to"].is_null() {
            desired
        } else {
            &event["to"]
        };
        let actual = state["groups"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|g| g["name"] == group)
            .map(|g| &g["now"]);
        if ["applied", "partial"].contains(&state["status"].as_str().unwrap_or(""))
            && actual.is_some_and(|a| expected.is_null() || a == expected)
        {
            event["status"] = json!("applied");
            event["confirmed_at"] = json!(crate::now());
            event["detail"] = Value::Null;
        }
    }
    d.data["selection_events"] = json!(history);
}
fn device_events(d: &store::Resource, b: &store::Resource) -> Vec<Value> {
    let mut history = events(d);
    for mut event in events(b) {
        if !history.iter().any(|e| e["id"] == event["id"]) {
            event["status"] = json!("pending");
            event["scope"] = json!("identity");
            history.push(event);
        }
    }
    history.sort_by_key(|e| e["created_at"].as_i64().unwrap_or(0));
    history.drain(..history.len().saturating_sub(80));
    history
}
pub fn jobs(d: &store::Resource) -> Vec<Job> {
    serde_json::from_value(d.data.get("rpc_jobs").cloned().unwrap_or(json!([]))).unwrap_or_default()
}
fn normalize(jobs: &mut Vec<Job>) {
    for j in jobs.iter_mut() {
        if ["queued", "executing"].contains(&j.status.as_str()) && j.expires_at <= crate::now() {
            j.status = if j.status == "executing" {
                "unknown"
            } else {
                "expired"
            }
            .into();
        }
    }
    while jobs.len() > 64 {
        let Some(index) = jobs
            .iter()
            .position(|j| !["queued", "executing"].contains(&j.status.as_str()))
        else {
            break;
        };
        jobs.remove(index);
    }
}
pub async fn enqueue(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<RpcRequest>,
) -> Result<Json<Job>, Error> {
    let user = auth::user(&app, &h, true).await?;
    auth::rate(&app, format!("rpc:{user}"), 60, 60).await?;
    req.validate().map_err(|e| Error::bad(e.to_string()))?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let mut d = store::get(&app, &mut tx, user, id).await?;
    if d.kind != "device" {
        return Err(Error::not_found());
    }
    if d.data["reported"]["protocol"].as_u64().unwrap_or(0) < 2 {
        return Err(Error::bad("请先升级 Agent 以使用设备控制协议 v2"));
    }
    let mut queue = jobs(&d);
    normalize(&mut queue);
    if let Some(j) = queue
        .iter()
        .find(|j| j.idempotency_key == req.idempotency_key && j.binding == binding(&d))
    {
        if j.method != req.method || j.params != req.params {
            return Err(Error::bad("idempotency key already used"));
        }
        return Ok(Json(j.clone()));
    }
    let now = crate::now();
    if ["core.start", "core.stop", "core.restart"].contains(&req.method.as_str()) {
        for j in &mut queue {
            if j.status == "queued"
                && ["core.start", "core.stop", "core.restart"].contains(&j.method.as_str())
            {
                j.status = "superseded".into();
            }
        }
    }
    let limit = if req.method.starts_with("core.") {
        16
    } else {
        14
    };
    if queue
        .iter()
        .filter(|j| ["queued", "executing"].contains(&j.status.as_str()))
        .count()
        >= limit
    {
        return Err(Error::new(StatusCode::TOO_MANY_REQUESTS, "设备队列已满"));
    }
    let job = Job {
        id: Uuid::new_v4().to_string(),
        protocol: 2,
        binding: binding(&d),
        method: req.method,
        params: req.params,
        idempotency_key: req.idempotency_key,
        expected_revision: d.data["reported"]["revision"].as_str().map(str::to_owned),
        created_at: now,
        expires_at: now + 600,
        status: "queued".into(),
        result: Value::Null,
    };
    queue.push(job.clone());
    normalize(&mut queue);
    d.data["rpc_jobs"] = json!(queue);
    store::put(&app, &mut tx, user, &d).await?;
    store::notify(&mut tx, user).await?;
    tx.commit().await?;
    tracing::info!(device_id = %d.id, job_id = %job.id, method = %job.method,
        "device control job queued");
    Ok(Json(job))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionEdit {
    pub expected_version: u64,
    pub selections: Selections,
}
async fn write_selection(
    app: &App,
    conn: &mut sqlx::PgConnection,
    user: Uuid,
    r: &mut store::Resource,
    e: SelectionEdit,
    source: &str,
) -> Result<(), Error> {
    camofy::protocol::validate_selections(&e.selections).map_err(|e| Error::bad(e.to_string()))?;
    if e.expected_version != version(r) {
        return Err(Error::new(
            StatusCode::CONFLICT,
            "节点选择已变更，请刷新后重试",
        ));
    }
    let field = if r.kind == "bundle" {
        "selections"
    } else if r.kind == "device" {
        "selection_overrides"
    } else {
        return Err(Error::not_found());
    };
    let previous = r.data[field].clone();
    if previous == json!(e.selections) {
        return Ok(());
    }
    r.data[field] = json!(e.selections);
    r.data["selection_version"] = json!(version(r) + 1);
    record_selection(r, &previous, source);
    r.version += 1;
    store::put(app, conn, user, r).await?;
    if r.kind == "bundle" {
        store::rebuild_selected(app, conn, user, Some(&[r.id])).await?;
    }
    store::notify(conn, user).await?;
    Ok(())
}
pub async fn select(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(e): Json<SelectionEdit>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, true).await?;
    auth::rate(&app, format!("select:{user}"), 120, 60).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let mut r = store::get(&app, &mut tx, user, id).await?;
    if r.kind != "bundle" {
        return Err(Error::bad("节点选择由身份统一管理，请在身份页面修改"));
    }
    write_selection(&app, &mut tx, user, &mut r, e, "cloud").await?;
    tx.commit().await?;
    tracing::info!(bundle_id = %id, version = version(&r), "identity selection committed");
    Ok(Json(json!({"version":version(&r)})))
}

pub async fn view(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, false).await?;
    let mut conn = app.db.acquire().await?;
    let r = store::get(&app, &mut conn, user, id).await?;
    let device = if r.kind == "device" {
        Some(r.clone())
    } else {
        None
    };
    let r = if r.kind == "device" {
        let bundle = Uuid::parse_str(r.data["bundle_id"].as_str().unwrap_or(""))
            .map_err(|_| Error::not_found())?;
        store::get(&app, &mut conn, user, bundle).await?
    } else {
        r
    };
    if r.kind != "bundle" {
        return Err(Error::not_found());
    }
    drop(conn);
    let (_, artifacts, _) = sync::current(&app, user, r.id, &r).await?;
    let content = artifacts["agent"]["content"]
        .as_str()
        .or(artifacts["router"]["content"].as_str())
        .ok_or_else(Error::not_found)?;
    let yaml = camofy::engine::parse(content)?;
    let mut groups = Vec::new();
    for g in yaml["proxy-groups"].as_sequence().into_iter().flatten() {
        groups.push(Group {
            name: g["name"].as_str().unwrap_or("").into(),
            kind: if g["type"] == "select" {
                "Selector".into()
            } else {
                g["type"].as_str().unwrap_or("").into()
            },
            members: g["proxies"]
                .as_sequence()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            now: None,
            dynamic: g.get("use").is_some()
                || g["include-all"].as_bool() == Some(true)
                || g["include-all-providers"].as_bool() == Some(true),
        });
    }
    let mut conn = app.db.acquire().await?;
    let devices:Vec<_>=store::list(&app,&mut conn,user).await?.into_iter().filter(|d|d.kind=="device"&&d.data["bundle_id"]==r.id.to_string()).map(|d|json!({"id":d.id,"name":d.data["name"],"reported":d.data["reported"],"overrides":{}})).collect();
    // Dynamic provider membership is evidence from devices, not guessed from YAML.
    for g in &mut groups {
        if g.dynamic {
            let mut extras = std::collections::BTreeSet::new();
            for d in &devices {
                if let Some(all) = d["reported"]["proxy_state"]["groups"].as_array() {
                    for runtime in all {
                        if runtime["name"] == g.name {
                            for member in runtime["members"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                            {
                                if !g.members.iter().any(|n| n == member) {
                                    extras.insert(member.to_owned());
                                }
                            }
                        }
                    }
                }
            }
            g.members.extend(extras);
        }
    }
    if let Some(d) = device {
        let runtime = &d.data["reported"]["proxy_state"]["groups"];
        for g in &mut groups {
            g.now = runtime
                .as_array()
                .into_iter()
                .flatten()
                .find(|p| p["name"] == g.name)
                .and_then(|p| p["now"].as_str())
                .map(str::to_owned);
        }
        let mut queue = jobs(&d);
        normalize(&mut queue);
        return Ok(Json(
            json!({"groups":groups,"state":d.data["reported"]["proxy_state"],"reported":d.data["reported"],"selections":r.data["selections"],"overrides":{},"version":version(&r),"jobs":queue,"events":device_events(&d,&r),"binding":binding(&d),"identity_id":r.id,"identity_name":r.data["name"]}),
        ));
    }
    Ok(Json(
        json!({"groups":groups,"selections":r.data["selections"],"overrides":{},"version":version(&r),"devices":devices,"events":events(&r)}),
    ))
}

#[derive(Deserialize)]
pub struct AgentReport {
    pub binding: String,
    pub state: Option<Value>,
    pub job_id: Option<String>,
    pub status: Option<String>,
    pub result: Option<Value>,
}
pub async fn agent_report(
    State(app): State<App>,
    h: HeaderMap,
    Json(body): Json<AgentReport>,
) -> Result<StatusCode, Error> {
    let a = sync::access(&app, auth::bearer(&h).ok_or_else(Error::unauthorized)?).await?;
    let id = a.device.ok_or_else(Error::unauthorized)?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, a.user).await?;
    let mut d = store::get(&app, &mut tx, a.user, id).await?;
    if body.binding != binding(&d) || d.data["bundle_id"] != a.bundle.to_string() {
        return Err(Error::new(StatusCode::CONFLICT, "device binding changed"));
    }
    if let Some(state) = body.state {
        if serde_json::to_vec(&state)?.len() > 512 * 1024 {
            return Err(Error::bad("state too large"));
        }
        let groups: Vec<Group> = serde_json::from_value(state["groups"].clone())
            .map_err(|_| Error::bad("invalid groups"))?;
        if groups.len() > 256 {
            return Err(Error::bad("too many groups"));
        }
        if !d.data["reported"].is_object() {
            d.data["reported"] = json!({});
        }
        d.data["reported"]["proxy_state"] = state;
        d.data["reported"]["proxy_state"]["received_at"] = json!(crate::now());
        d.data["reported"]["protocol"] = json!(2);
        let identity = store::get(&app, &mut tx, a.user, a.bundle).await?;
        d.data["selection_events"] = json!(device_events(&d, &identity));
        d.data["selection_overrides"] = json!({});
        settle_events(&mut d);
    }
    if let Some(id) = body.job_id.as_deref() {
        let mut queue = jobs(&d);
        normalize(&mut queue);
        let j = queue
            .iter_mut()
            .find(|j| j.id == id && j.binding == body.binding)
            .ok_or_else(Error::not_found)?;
        let status = body.status.as_deref().unwrap_or("");
        if status == "executing" && !["queued", "executing"].contains(&j.status.as_str()) {
            return Err(Error::new(
                StatusCode::CONFLICT,
                "job is no longer executable",
            ));
        }
        if !["executing", "succeeded", "failed", "unknown"].contains(&status) {
            return Err(Error::bad("invalid job status"));
        }
        if ["queued", "executing"].contains(&j.status.as_str()) {
            let result = body.result.unwrap_or(Value::Null);
            if serde_json::to_vec(&result)?.len() > 64 * 1024 {
                return Err(Error::bad("result too large"));
            }
            j.status = status.into();
            j.result = result;
        }
        d.data["rpc_jobs"] = json!(queue);
    }
    store::put(&app, &mut tx, a.user, &d).await?;
    tx.commit().await?;
    if body.status.as_deref() == Some("failed") || body.status.as_deref() == Some("unknown") {
        tracing::warn!(device_id = %id, job_id = ?body.job_id, status = ?body.status,
            "device control job reported failure");
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct OverrideEdit {
    pub binding: String,
    pub expected_version: u64,
    pub selections: Selections,
}
pub async fn agent_overrides(
    State(app): State<App>,
    h: HeaderMap,
    Json(e): Json<OverrideEdit>,
) -> Result<Json<Value>, Error> {
    let a = sync::access(&app, auth::bearer(&h).ok_or_else(Error::unauthorized)?).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, a.user).await?;
    let mut d = store::get(
        &app,
        &mut tx,
        a.user,
        a.device.ok_or_else(Error::unauthorized)?,
    )
    .await?;
    if e.binding != binding(&d) || d.data["bundle_id"] != a.bundle.to_string() {
        return Err(Error::new(StatusCode::CONFLICT, "device binding changed"));
    }
    if !e.selections.is_empty() {
        return Err(Error::bad("设备覆盖已停用，请在身份页面修改选择"));
    }
    write_selection(
        &app,
        &mut tx,
        a.user,
        &mut d,
        SelectionEdit {
            expected_version: e.expected_version,
            selections: e.selections,
        },
        "local",
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"version":version(&d)})))
}
