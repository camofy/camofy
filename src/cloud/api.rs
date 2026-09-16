use crate::{
    App, Error, auth,
    store::{self, Resource},
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub async fn resources(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, false).await?;
    let mut conn = app.db.acquire().await?;
    let mut records = store::list(&app, &mut conn, user).await?;
    for r in &mut records {
        if r.kind == "bundle" {
            r.data["system_profile"] = store::system_profile(&app.origin)?;
        }
        if r.kind == "proxy" {
            crate::provider::redact(&mut r.data);
        }
        if r.kind == "profile" && r.data["type"] == "source" {
            r.data.as_object_mut().unwrap().remove("content");
        }
    }
    Ok(Json(json!(records)))
}

#[derive(Deserialize)]
pub struct Edit {
    pub kind: String,
    pub version: Option<i64>,
    pub data: Value,
}
pub async fn create(
    State(app): State<App>,
    h: HeaderMap,
    Json(e): Json<Edit>,
) -> Result<Json<Resource>, Error> {
    save(app, h, Uuid::new_v4(), e, true).await
}
pub async fn update(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(e): Json<Edit>,
) -> Result<Json<Resource>, Error> {
    save(app, h, id, e, false).await
}

async fn save(
    app: App,
    h: HeaderMap,
    id: Uuid,
    mut e: Edit,
    new: bool,
) -> Result<Json<Resource>, Error> {
    let user = auth::user(&app, &h, true).await?;
    auth::rate(&app, format!("edit:{user}"), 120, 60).await?;
    if !["proxy", "profile", "bundle", "device"].contains(&e.kind.as_str()) {
        return Err(Error::bad("unknown resource kind"));
    }
    if e.kind == "proxy" {
        if !e.data.is_object()
            || e.data["name"]
                .as_str()
                .is_none_or(|s| s.trim().is_empty() || s.len() > 120)
        {
            return Err(Error::bad("proxy data and name required"));
        }
        let old = if new {
            None
        } else {
            let mut conn = app.db.acquire().await?;
            let r = store::get(&app, &mut conn, user, id).await?;
            if r.kind != "proxy" || e.version != Some(r.version) {
                return Err(Error::new(
                    StatusCode::CONFLICT,
                    "resource changed; reload before saving",
                ));
            }
            Some(r)
        };
        crate::provider::provision(&app, user, &mut e.data, old.as_ref().map(|r| &r.data))
            .await
            .map_err(|e| Error::bad(e.to_string()))?;
    }
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let records = store::list(&app, &mut tx, user).await?;
    let old = if new {
        None
    } else {
        Some(store::get(&app, &mut tx, user, id).await?)
    };
    if let Some(r) = &old
        && (r.kind != e.kind || e.version != Some(r.version))
    {
        return Err(Error::new(
            StatusCode::CONFLICT,
            "resource changed; reload before saving",
        ));
    }
    let mut data = e.data;
    let object = data
        .as_object_mut()
        .ok_or_else(|| Error::bad("data must be an object"))?;
    object.remove("_package");
    if new && (object.contains_key("store") || object.get("origin").is_some_and(|v| v == "store")) {
        return Err(Error::bad("install managed profiles through the store"));
    }
    if let Some(old) = &old {
        if old.data["store"].is_object() {
            if data["content"] != old.data["content"]
                || data["store"] != old.data["store"]
                || data["type"] != old.data["type"]
            {
                return Err(Error::bad(
                    "managed content is immutable; use upgrade or fork",
                ));
            }
            data["origin"] = json!("store");
        } else if data.get("store").is_some() || data["origin"] == "store" {
            return Err(Error::bad("cannot forge managed profile metadata"));
        }
    }
    let object = data.as_object_mut().unwrap();
    for key in [
        "error",
        "last_fetch",
        "fetch_status",
        "published_revision",
        "published_hash",
        "outputs",
        "reported",
        "command",
        "subscription_url",
        "last_proxy",
        "last_proxy_at",
        "system_profile",
    ] {
        object.remove(key);
        if let Some(v) = old.as_ref().and_then(|r| r.data.get(key)) {
            object.insert(key.into(), v.clone());
        }
    }
    let name = data["name"].as_str().unwrap_or("");
    if name.trim().is_empty() || name.len() > 120 {
        return Err(Error::bad("name must contain 1–120 bytes"));
    }
    let reference = |value: &Value, kind: &str| -> Result<(), Error> {
        if !records
            .iter()
            .any(|r| r.kind == kind && Some(r.id.to_string()).as_deref() == value.as_str())
        {
            return Err(Error::bad(format!(
                "{kind} reference must belong to your account"
            )));
        }
        Ok(())
    };
    match e.kind.as_str() {
        "proxy" => {
            // Validated and provisioned above, outside the tenant transaction.
        }
        "profile" => {
            if data.get("enabled").is_some() || data.get("is_active").is_some() {
                return Err(Error::bad(
                    "profile activation belongs to an identity binding",
                ));
            }
            let t = data["type"].as_str().unwrap_or("").to_string();
            if let Some(o) = &old
                && o.data["type"] != t
            {
                return Err(Error::bad("profile type cannot be changed"));
            }
            if t == "source" {
                let u = url::Url::parse(
                    data["url"]
                        .as_str()
                        .ok_or_else(|| Error::bad("subscription URL required"))?,
                )
                .map_err(|_| Error::bad("invalid subscription URL"))?;
                if !["http", "https"].contains(&u.scheme())
                    || u.host_str().is_none()
                    || !u.username().is_empty()
                    || u.password().is_some()
                {
                    return Err(Error::bad(
                        "subscription must use HTTP(S) without URL userinfo",
                    ));
                }
                if !data["proxy_id"].is_null() {
                    reference(&data["proxy_id"], "proxy")?;
                }
                let interval = data["interval_seconds"].as_i64().unwrap_or(3600);
                if !(300..=604800).contains(&interval) {
                    return Err(Error::bad("refresh interval must be 300–604800 seconds"));
                }
                data["interval_seconds"] = json!(interval);
                // Source content is exclusively written by the fetch worker.
                data["content"] = old
                    .as_ref()
                    .map(|r| r.data["content"].clone())
                    .unwrap_or(Value::Null);
            } else if t == "overlay" {
                if !data["store"].is_object() {
                    camofy::engine::parse(
                        data["content"]
                            .as_str()
                            .ok_or_else(|| Error::bad("overlay YAML required"))?,
                    )
                    .map_err(|e| Error::bad(e.to_string()))?;
                }
            } else {
                return Err(Error::bad("profile type must be source or overlay"));
            }
        }
        "bundle" => {
            let bindings = data["profiles"].as_array().ok_or_else(|| {
                Error::bad("profiles must be ordered {profile_id, enabled} bindings")
            })?;
            let mut seen = std::collections::HashSet::new();
            for binding in bindings {
                let p = &binding["profile_id"];
                reference(p, "profile")?;
                if !seen.insert(p.to_string()) {
                    return Err(Error::bad("duplicate profile binding"));
                }
                if !binding["enabled"].is_boolean() {
                    return Err(Error::bad("each profile binding needs an enabled boolean"));
                }
            }
            data.as_object_mut().unwrap().remove("source_id");
            data.as_object_mut().unwrap().remove("overlays");
            if data.get("selections").is_none() {
                data["selections"] = json!({});
            }
            serde_json::from_value::<std::collections::BTreeMap<String, String>>(
                data["selections"].clone(),
            )
            .map_err(|_| Error::bad("selections must map group names to node names"))?;
        }
        "device" => {
            reference(&data["bundle_id"], "bundle")?;
            if old
                .as_ref()
                .is_some_and(|o| o.data["bundle_id"] != data["bundle_id"])
            {
                // Credentials identify the device, not a manually installed subscription.
                sqlx::query(
                    "UPDATE access_tokens SET bundle_id=$3 WHERE user_id=$1 AND device_id=$2",
                )
                .bind(user)
                .bind(id)
                .bind(
                    Uuid::parse_str(data["bundle_id"].as_str().unwrap())
                        .map_err(|_| Error::bad("invalid identity"))?,
                )
                .execute(&mut *tx)
                .await?;
                data["command"] = Value::Null;
                data["reported"] = Value::Null;
            }
        }
        _ => unreachable!(),
    }
    let mut r = Resource {
        id,
        kind: e.kind,
        version: old.map(|r| r.version + 1).unwrap_or(1),
        data,
    };
    store::put(&app, &mut tx, user, &r).await?;
    // A managed identity edit must be valid before its association can be committed.
    if r.kind == "bundle"
        && r.data["profiles"].as_array().is_some_and(|bs| {
            bs.iter().any(|b| {
                b["enabled"] == true
                    && records.iter().any(|p| {
                        p.id.to_string() == b["profile_id"].as_str().unwrap_or("")
                            && p.data["store"].is_object()
                    })
            })
        })
    {
        store::render_bundle(&records, &r.data, &app.origin)
            .map_err(|e| Error::bad(e.to_string()))?;
    }
    if new && r.kind == "bundle" {
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        sqlx::query("INSERT INTO access_tokens(hash,user_id,bundle_id,label) VALUES($1,$2,$3,$4)")
            .bind(camofy::digest(&token))
            .bind(user)
            .bind(id)
            .bind("Identity subscription")
            .execute(&mut *tx)
            .await?;
        r.data["subscription_url"] = json!(format!("{}/sub/{token}", app.origin));
        store::put(&app, &mut tx, user, &r).await?;
    }
    if r.kind == "profile" && r.data["type"] == "source" {
        // Save schedules even when auto refresh is off: one initial/manual refresh is allowed.
        if new || r.data["auto_refresh"].as_bool().unwrap_or(true) {
            sqlx::query("INSERT INTO fetch_jobs(profile_id,user_id) VALUES($1,$2) ON CONFLICT(profile_id) DO UPDATE SET next_run=now()").bind(id).bind(user).execute(&mut *tx).await?;
        } else {
            sqlx::query("DELETE FROM fetch_jobs WHERE profile_id=$1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }
    if r.kind == "profile" || r.kind == "bundle" {
        store::rebuild(&app, &mut tx, user).await?;
    }
    if r.kind == "device" {
        store::notify(&mut tx, user).await?;
    }
    r = store::get(&app, &mut tx, user, id).await?;
    tx.commit().await?;
    if r.kind == "proxy" {
        crate::provider::redact(&mut r.data);
    }
    if r.kind == "profile" && r.data["type"] == "source" {
        r.data.as_object_mut().unwrap().remove("content");
    }
    Ok(Json(r))
}

pub async fn delete(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    let user = auth::user(&app, &h, true).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    store::get(&app, &mut tx, user, id).await?;
    for r in store::list(&app, &mut tx, user).await? {
        if ["proxy_id", "bundle_id"]
            .iter()
            .any(|k| r.data[*k] == id.to_string())
            || r.data["profiles"].as_array().is_some_and(|a| {
                a.iter()
                    .any(|binding| binding["profile_id"] == id.to_string())
            })
        {
            return Err(Error::new(
                StatusCode::CONFLICT,
                "resource is referenced; remove its bindings first",
            ));
        }
    }
    sqlx::query("DELETE FROM resources WHERE id=$1 AND user_id=$2")
        .bind(id)
        .bind(user)
        .execute(&mut *tx)
        .await?;
    store::notify(&mut tx, user).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
/// Read the tenant's last successful profile snapshot without fetching upstream.
pub async fn profile_content(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, false).await?;
    let mut conn = app.db.acquire().await?;
    let r = store::get(&app, &mut conn, user, id).await?;
    if r.kind != "profile" {
        return Err(Error::not_found());
    }
    let content = r.data["content"]
        .as_str()
        .ok_or_else(|| Error::new(StatusCode::CONFLICT, "尚无成功拉取的内容，请先刷新订阅。"))?;
    Ok(Json(
        json!({"content":content,"version":r.version,"last_fetch":r.data["last_fetch"]}),
    ))
}

pub async fn refresh(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    let user = auth::user(&app, &h, true).await?;
    auth::rate(&app, format!("refresh:{user}"), 20, 60).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let r = store::get(&app, &mut tx, user, id).await?;
    if r.kind != "profile" || r.data["type"] != "source" {
        return Err(Error::bad("not a subscription profile"));
    }
    sqlx::query("INSERT INTO fetch_jobs(profile_id,user_id) VALUES($1,$2) ON CONFLICT(profile_id) DO UPDATE SET next_run=now()").bind(id).bind(user).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
pub struct TokenRequest {
    bundle_id: Uuid,
    device_id: Option<Uuid>,
    label: String,
}
pub async fn token(
    State(app): State<App>,
    h: HeaderMap,
    Json(t): Json<TokenRequest>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, true).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let mut bundle = store::get(&app, &mut tx, user, t.bundle_id).await?;
    if bundle.kind != "bundle" {
        return Err(Error::bad("bundle required"));
    }
    if let Some(id) = t.device_id {
        let d = store::get(&app, &mut tx, user, id).await?;
        if d.kind != "device" || d.data["bundle_id"] != t.bundle_id.to_string() {
            return Err(Error::bad("device binding mismatch"));
        }
        sqlx::query("DELETE FROM access_tokens WHERE device_id=$1 AND user_id=$2")
            .bind(id)
            .bind(user)
            .execute(&mut *tx)
            .await?;
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM access_tokens WHERE user_id=$1")
        .bind(user)
        .fetch_one(&mut *tx)
        .await?;
    if n >= 100 {
        return Err(Error::bad("token limit reached"));
    }
    if t.label.is_empty() || t.label.len() > 120 {
        return Err(Error::bad("token label required (up to 120 bytes)"));
    }
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO access_tokens(hash,user_id,bundle_id,device_id,label) VALUES($1,$2,$3,$4,$5)",
    )
    .bind(camofy::digest(&token))
    .bind(user)
    .bind(t.bundle_id)
    .bind(t.device_id)
    .bind(t.label)
    .execute(&mut *tx)
    .await?;
    if t.device_id.is_none() && !bundle.data["subscription_url"].is_string() {
        bundle.data["subscription_url"] = json!(format!("{}/sub/{token}", app.origin));
        bundle.version += 1;
        store::put(&app, &mut tx, user, &bundle).await?;
    }
    tx.commit().await?;
    Ok(Json(
        json!({"token":token,"cloud_url":app.origin,"subscription_base":format!("{}/sub/{token}",app.origin)}),
    ))
}
pub async fn tokens(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>, Error> {
    use sqlx::Row;
    let user = auth::user(&app, &h, false).await?;
    let rows=sqlx::query("SELECT hash,bundle_id,device_id,label FROM access_tokens WHERE user_id=$1 ORDER BY created_at DESC").bind(user).fetch_all(&app.db).await?;
    Ok(Json(json!(rows.into_iter().map(|r|json!({"id":r.get::<String,_>("hash"),"bundle_id":r.get::<Uuid,_>("bundle_id"),"device_id":r.get::<Option<Uuid>,_>("device_id"),"label":r.get::<String,_>("label")})).collect::<Vec<_>>())))
}
pub async fn revoke(
    State(app): State<App>,
    h: HeaderMap,
    Path(hash): Path<String>,
) -> Result<StatusCode, Error> {
    let user = auth::user(&app, &h, true).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    for mut r in store::list(&app, &mut tx, user).await? {
        if r.kind == "bundle"
            && r.data["subscription_url"]
                .as_str()
                .and_then(|u| url::Url::parse(u).ok())
                .and_then(|u| {
                    u.path_segments()
                        .and_then(|mut s| s.nth(1))
                        .map(str::to_owned)
                })
                .is_some_and(|token| camofy::digest(token) == hash)
        {
            r.data["subscription_url"] = Value::Null;
            r.version += 1;
            store::put(&app, &mut tx, user, &r).await?;
        }
    }
    sqlx::query("DELETE FROM access_tokens WHERE user_id=$1 AND hash=$2")
        .bind(user)
        .bind(hash)
        .execute(&mut *tx)
        .await?;
    store::notify(&mut tx, user).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn revisions(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Error> {
    use sqlx::Row;
    let user = auth::user(&app, &h, false).await?;
    let rows=sqlx::query("SELECT id,created_at::text FROM revisions WHERE user_id=$1 AND bundle_id=$2 ORDER BY created_at DESC").bind(user).bind(id).fetch_all(&app.db).await?;
    Ok(Json(json!(
        rows.iter()
            .map(|r| json!({"id":r.get::<Uuid,_>(0),"created_at":r.get::<String,_>(1)}))
            .collect::<Vec<_>>()
    )))
}
#[derive(Deserialize)]
pub struct Rollback {
    revision: Uuid,
}
pub async fn rollback(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<Rollback>,
) -> Result<StatusCode, Error> {
    let user = auth::user(&app, &h, true).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let mut b = store::get(&app, &mut tx, user, id).await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM revisions WHERE id=$1 AND user_id=$2 AND bundle_id=$3)",
    )
    .bind(body.revision)
    .bind(user)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if b.kind != "bundle" || !exists {
        return Err(Error::not_found());
    }
    let artifacts: Value = sqlx::query_scalar(
        "SELECT artifacts FROM revisions WHERE id=$1 AND user_id=$2 AND bundle_id=$3",
    )
    .bind(body.revision)
    .bind(user)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let artifacts = app.vault.open(artifacts)?;
    let yaml = camofy::engine::parse(artifacts["router"]["content"].as_str().unwrap_or("{}"))?;
    let safety = camofy::engine::cloud_safety_profile(&app.origin)?;
    if yaml["mode"] != "rule" || yaml["rules"][0] != safety["prepend-rules"][0] {
        return Err(Error::bad(
            "此历史版本缺少当前云端直连保护，不能直接回滚；请通过 Profile 重新发布",
        ));
    }
    b.data["published_revision"] = json!(body.revision);
    b.data["published_hash"] = Value::Null;
    b.version += 1;
    store::put(&app, &mut tx, user, &b).await?;
    store::notify(&mut tx, user).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn preview(
    State(app): State<App>,
    h: HeaderMap,
    Path((id, format)): Path<(Uuid, String)>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, false).await?;
    let mut conn = app.db.acquire().await?;
    let b = store::get(&app, &mut conn, user, id).await?;
    drop(conn);
    let revision = crate::sync::current(&app, user, id, &b).await?;
    Ok(Json(revision.1[&format].clone()))
}

pub async fn test_device(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, true).await?;
    auth::rate(&app, format!("test:{user}"), 6, 60).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let mut d = store::get(&app, &mut tx, user, id).await?;
    if d.kind != "device" {
        return Err(Error::bad("device required"));
    }
    if d.data["command"]["expires_at"].as_u64().unwrap_or(0) > crate::now() {
        return Err(Error::new(
            StatusCode::CONFLICT,
            "设备仍有待执行指令，请等待回执或过期后重试",
        ));
    }
    let command = json!({"id":Uuid::new_v4(),"type":"test_delays","expires_at":crate::now()+120});
    d.data["command"] = command.clone();
    store::put(&app, &mut tx, user, &d).await?;
    store::notify(&mut tx, user).await?;
    tx.commit().await?;
    Ok(Json(command))
}

#[derive(Deserialize)]
pub struct Control {
    action: String,
}
pub async fn control_device(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(c): Json<Control>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, true).await?;
    if !["start", "stop", "restart"].contains(&c.action.as_str()) {
        return Err(Error::bad("unknown core action"));
    }
    auth::rate(&app, format!("control:{user}"), 30, 60).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let mut d = store::get(&app, &mut tx, user, id).await?;
    if d.kind != "device" {
        return Err(Error::bad("device required"));
    }
    if d.data["command"]["expires_at"].as_u64().unwrap_or(0) > crate::now() {
        return Err(Error::new(
            StatusCode::CONFLICT,
            "设备仍有待执行指令，请等待回执或过期后重试",
        ));
    }
    let command = json!({"id":Uuid::new_v4(),"type":c.action,"expires_at":crate::now()+120});
    d.data["command"] = command.clone();
    store::put(&app, &mut tx, user, &d).await?;
    store::notify(&mut tx, user).await?;
    tx.commit().await?;
    Ok(Json(command))
}
