//! A subscription source's own client link: a hidden identity that binds only that source.
//! Publication, formats, devices, selections and history all reuse the identity pipeline.
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
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

/// The source a hidden identity belongs to. Server-owned; never accepted from clients.
pub fn source_of(r: &Resource) -> Option<&str> {
    (r.kind == "bundle")
        .then(|| r.data["managed_source"].as_str())
        .flatten()
}

pub fn find(records: &[Resource], source: Uuid) -> Option<&Resource> {
    records
        .iter()
        .find(|r| source_of(r) == Some(source.to_string().as_str()))
}

/// Create the hidden identity with a primary link. The caller publishes it, if needed.
pub async fn issue(
    app: &App,
    conn: &mut PgConnection,
    user: Uuid,
    source: &Resource,
) -> Result<Resource, Error> {
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let bundle = Resource {
        id: Uuid::new_v4(),
        kind: "bundle".into(),
        version: 1,
        data: json!({
            "name": source.data["name"],
            "profiles": [{"profile_id": source.id, "enabled": true}],
            "selections": {},
            "selection_version": 1,
            "managed_source": source.id,
            "subscription_url": format!("{}/sub/{token}", app.origin),
        }),
    };
    store::put(app, conn, user, &bundle).await?;
    sqlx::query("INSERT INTO access_tokens(hash,user_id,bundle_id,label) VALUES($1,$2,$3,$4)")
        .bind(camofy::digest(&token))
        .bind(user)
        .bind(bundle.id)
        .bind("Identity subscription")
        .execute(&mut *conn)
        .await?;
    Ok(bundle)
}

/// Generate a link for a source created before links were automatic. Idempotent.
pub async fn create(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Resource>, Error> {
    let user = auth::user(&app, &h, true).await?;
    auth::rate(&app, format!("edit:{user}"), 120, 60).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let records = store::list(&app, &mut tx, user).await?;
    let source = records
        .iter()
        .find(|r| r.id == id && r.kind == "profile" && r.data["type"] == "source")
        .ok_or_else(Error::not_found)?;
    let bundle_id = match find(&records, id) {
        Some(existing) => existing.id,
        None => {
            let bundle = issue(&app, &mut tx, user, source).await?;
            store::rebuild_selected(&app, &mut tx, user, Some(&[bundle.id])).await?;
            store::notify(&mut tx, user).await?;
            tracing::info!(profile_id = %id, bundle_id = %bundle.id, "source link issued");
            bundle.id
        }
    };
    let bundle = store::get(&app, &mut tx, user, bundle_id).await?;
    tx.commit().await?;
    Ok(Json(bundle))
}

#[derive(Deserialize)]
pub struct Promote {
    version: i64,
}

/// Turn a source link into an ordinary identity. Its links, devices and history are kept.
pub async fn promote(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(p): Json<Promote>,
) -> Result<Json<Resource>, Error> {
    let user = auth::user(&app, &h, true).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let mut bundle = store::get(&app, &mut tx, user, id).await?;
    if source_of(&bundle).is_none() {
        return Err(Error::not_found());
    }
    if bundle.version != p.version {
        return Err(Error::new(
            StatusCode::CONFLICT,
            "identity changed; reload before converting",
        ));
    }
    bundle
        .data
        .as_object_mut()
        .unwrap()
        .remove("managed_source");
    bundle.version += 1;
    store::put(&app, &mut tx, user, &bundle).await?;
    store::notify(&mut tx, user).await?;
    let bundle = store::get(&app, &mut tx, user, id).await?;
    tx.commit().await?;
    tracing::info!(bundle_id = %id, "source link promoted to identity");
    Ok(Json(bundle))
}
