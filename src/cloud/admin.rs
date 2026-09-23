//! Platform egress is never selected by tenant input. Secrets remain encrypted at rest.
use crate::{App, Error, auth, provider, store::Resource};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

pub async fn lock(conn: &mut PgConnection) -> Result<(), Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(739214801)")
        .execute(conn)
        .await?;
    Ok(())
}
pub async fn list_records(app: &App, conn: &mut PgConnection) -> Result<Vec<Resource>, Error> {
    sqlx::query("SELECT id,version,data FROM platform_proxies ORDER BY updated_at,id")
        .fetch_all(conn)
        .await?
        .into_iter()
        .map(|row| {
            let mut data = app.vault.open(row.get("data"))?;
            provider::redact(&mut data);
            Ok(Resource {
                id: row.get("id"),
                kind: "proxy".into(),
                version: row.get("version"),
                data,
            })
        })
        .collect()
}
pub async fn list(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>, Error> {
    auth::admin(&app, &h, false).await?;
    Ok(Json(json!(
        list_records(&app, &mut *app.db.acquire().await?).await?
    )))
}
async fn get(app: &App, conn: &mut PgConnection, id: Uuid) -> Result<Resource, Error> {
    let r = sqlx::query("SELECT version,data FROM platform_proxies WHERE id=$1")
        .bind(id)
        .fetch_optional(conn)
        .await?
        .ok_or_else(Error::not_found)?;
    Ok(Resource {
        id,
        kind: "proxy".into(),
        version: r.get("version"),
        data: app.vault.open(r.get("data"))?,
    })
}
pub async fn audit(
    conn: &mut PgConnection,
    actor: Uuid,
    action: &str,
    id: Option<Uuid>,
    version: i64,
) -> Result<(), Error> {
    sqlx::query(
        "INSERT INTO admin_audit_events(actor_id,action,object_id,version) VALUES($1,$2,$3,$4)",
    )
    .bind(actor)
    .bind(action)
    .bind(id)
    .bind(version)
    .execute(conn)
    .await?;
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    version: Option<i64>,
    data: Value,
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
    let actor = auth::admin(&app, &h, true).await?;
    auth::rate(&app, format!("admin-proxy:{actor}"), 20, 60).await?;
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
        Some(get(&app, &mut *app.db.acquire().await?, id).await?)
    };
    if old.as_ref().is_some_and(|r| Some(r.version) != e.version) {
        return Err(conflict());
    }
    provider::provision(&app, actor, &mut e.data, old.as_ref().map(|r| &r.data))
        .await
        .map_err(|_| Error::bad("代理配置或白名单确认失败，请重新预览并检查供应商配置。"))?;
    let mut tx = app.db.begin().await?;
    lock(&mut tx).await?;
    auth::admin(&app, &h, true).await?;
    if let Some(old) = &old
        && get(&app, &mut tx, id).await?.version != old.version
    {
        return Err(conflict());
    }
    let version = old.as_ref().map_or(1, |r| r.version + 1);
    sqlx::query("INSERT INTO platform_proxies(id,data,version,updated_by) VALUES($1,$2,$3,$4) ON CONFLICT(id) DO UPDATE SET data=EXCLUDED.data,version=EXCLUDED.version,updated_by=EXCLUDED.updated_by,updated_at=now()")
        .bind(id).bind(app.vault.seal(&e.data)?).bind(version).bind(actor).execute(&mut *tx).await?;
    audit(
        &mut tx,
        actor,
        if new { "proxy.create" } else { "proxy.update" },
        Some(id),
        version,
    )
    .await?;
    tx.commit().await?;
    provider::redact(&mut e.data);
    Ok(Json(Resource {
        id,
        kind: "proxy".into(),
        version,
        data: e.data,
    }))
}
fn conflict() -> Error {
    Error::new(StatusCode::CONFLICT, "配置已变更，请刷新后重试。")
}
pub async fn delete(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    let actor = auth::admin(&app, &h, true).await?;
    let mut tx = app.db.begin().await?;
    lock(&mut tx).await?;
    let active: Option<Uuid> =
        sqlx::query_scalar("SELECT proxy_id FROM subscription_egress_policy WHERE singleton")
            .fetch_one(&mut *tx)
            .await?;
    if active == Some(id) {
        return Err(Error::new(
            StatusCode::CONFLICT,
            "请先切换或暂停全局出口，再删除代理。",
        ));
    }
    let old = get(&app, &mut tx, id).await?;
    sqlx::query("DELETE FROM platform_proxies WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, actor, "proxy.delete", Some(id), old.version).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn policy(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>, Error> {
    auth::admin(&app, &h, false).await?;
    let row = sqlx::query(
        "SELECT proxy_id,direct,version FROM subscription_egress_policy WHERE singleton",
    )
    .fetch_one(&app.db)
    .await?;
    Ok(Json(
        json!({"proxy_id":row.get::<Option<Uuid>,_>("proxy_id"),"direct":row.get::<bool,_>("direct"),"version":row.get::<i64,_>("version")}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyEdit {
    proxy_id: Option<Uuid>,
    #[serde(default)]
    direct: bool,
    version: i64,
}
pub async fn set_policy(
    State(app): State<App>,
    h: HeaderMap,
    Json(e): Json<PolicyEdit>,
) -> Result<Json<Value>, Error> {
    let actor = auth::admin(&app, &h, true).await?;
    if e.direct && e.proxy_id.is_some() {
        return Err(Error::bad("直连模式不能同时选择代理"));
    }
    let mut tx = app.db.begin().await?;
    lock(&mut tx).await?;
    if let Some(id) = e.proxy_id {
        get(&app, &mut tx, id).await?;
    }
    let version: Option<i64>=sqlx::query_scalar("UPDATE subscription_egress_policy SET proxy_id=$1,direct=$2,version=version+1,updated_by=$4,updated_at=now() WHERE singleton AND version=$3 RETURNING version")
        .bind(e.proxy_id).bind(e.direct).bind(e.version).bind(actor).fetch_optional(&mut *tx).await?;
    let version = version.ok_or_else(conflict)?;
    audit(&mut tx, actor, "egress.select", e.proxy_id, version).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"proxy_id":e.proxy_id,"direct":e.direct,"version":version}),
    ))
}
#[derive(Clone)]
pub struct Snapshot {
    pub version: i64,
    pub direct: bool,
    pub proxy: Option<Resource>,
}
impl Snapshot {
    pub fn same(&self, other: &Self) -> bool {
        self.version == other.version
            && self.direct == other.direct
            && self.proxy.as_ref().map(|r| (r.id, r.version))
                == other.proxy.as_ref().map(|r| (r.id, r.version))
    }
}
pub async fn snapshot(app: &App, conn: &mut PgConnection) -> Result<Snapshot, Error> {
    let row=sqlx::query("SELECT e.version,e.direct,p.id,p.version AS proxy_version,p.data FROM subscription_egress_policy e LEFT JOIN platform_proxies p ON p.id=e.proxy_id WHERE singleton")
        .fetch_one(conn).await?;
    let proxy = if let Some(id) = row.get::<Option<Uuid>, _>("id") {
        Some(Resource {
            id,
            kind: "proxy".into(),
            version: row.get("proxy_version"),
            data: app.vault.open(row.get("data"))?,
        })
    } else {
        None
    };
    Ok(Snapshot {
        version: row.get("version"),
        direct: row.get("direct"),
        proxy,
    })
}
