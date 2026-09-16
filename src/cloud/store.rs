use crate::{App, Error};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
pub struct Resource {
    pub id: Uuid,
    pub kind: String,
    pub version: i64,
    pub data: Value,
}

/// Lazy, lossless upgrade of old identity links. No credential rotation or revision rebuild.
/// Subsequent writes persist the canonical form; old consumers keep their working alias.
fn canonical_data(mut data: Value, origin: &str) -> Value {
    if let Some(link) = data["subscription_url"].as_str()
        && let Ok(u) = url::Url::parse(link)
        && u.query().is_none()
        && u.fragment().is_none()
    {
        let parts: Vec<_> = u.path().split('/').collect();
        if (parts.len() == 3 || (parts.len() == 4 && parts[3] == "router"))
            && parts[1] == "sub"
            && parts[2].len() == 64
            && parts[2].bytes().all(|c| c.is_ascii_hexdigit())
        {
            data["subscription_url"] = json!(format!("{origin}/sub/{}", parts[2]));
        }
    }
    data
}

#[test]
fn legacy_identity_links_normalize_without_rotating_tokens() {
    let base = format!("https://cloud.example/sub/{}", "a".repeat(64));
    let old = json!({"subscription_url":format!("{base}/router"),"name":"identity","published_revision":"unchanged"});
    let new = canonical_data(old, "https://cloud.example");
    assert_eq!(new["subscription_url"], base);
    assert_eq!(new["published_revision"], "unchanged");
    assert_eq!(canonical_data(new.clone(), "https://cloud.example"), new);
    let migrated = canonical_data(new.clone(), "https://new.example");
    assert_eq!(
        migrated["subscription_url"],
        format!("https://new.example/sub/{}", "a".repeat(64))
    );
    assert_eq!(migrated["published_revision"], "unchanged");
    for link in [
        format!("{base}/clash"),
        format!("{base}/router?x=1"),
        "https://cloud.example/sub/invalid/router".into(),
    ] {
        let data = json!({"subscription_url":link});
        assert_eq!(canonical_data(data.clone(), "https://cloud.example"), data);
    }
}

pub async fn list(app: &App, conn: &mut PgConnection, user: Uuid) -> Result<Vec<Resource>, Error> {
    let rows = sqlx::query(
        "SELECT id,kind,version,data FROM resources WHERE user_id=$1 ORDER BY updated_at,id",
    )
    .bind(user)
    .fetch_all(&mut *conn)
    .await?;
    let mut records: Vec<Resource> = rows
        .into_iter()
        .map(|r| {
            Ok(Resource {
                id: r.get("id"),
                kind: r.get("kind"),
                version: r.get("version"),
                data: canonical_data(app.vault.open(r.get("data"))?, &app.origin),
            })
        })
        .collect::<Result<_, Error>>()?;
    crate::catalog::hydrate(conn, &mut records).await?;
    Ok(records)
}
pub async fn get(
    app: &App,
    conn: &mut PgConnection,
    user: Uuid,
    id: Uuid,
) -> Result<Resource, Error> {
    let row = sqlx::query("SELECT id,kind,version,data FROM resources WHERE user_id=$1 AND id=$2")
        .bind(user)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(Error::not_found)?;
    let mut record = Resource {
        id: row.get("id"),
        kind: row.get("kind"),
        version: row.get("version"),
        data: canonical_data(app.vault.open(row.get("data"))?, &app.origin),
    };
    crate::catalog::hydrate(conn, std::slice::from_mut(&mut record)).await?;
    Ok(record)
}
pub async fn put(
    app: &App,
    conn: &mut PgConnection,
    user: Uuid,
    r: &Resource,
) -> Result<(), Error> {
    let mut data = r.data.clone();
    data.as_object_mut().unwrap().remove("_package");
    sqlx::query("INSERT INTO resources(id,user_id,kind,data,version) VALUES($1,$2,$3,$4,$5) ON CONFLICT(id) DO UPDATE SET data=EXCLUDED.data,version=EXCLUDED.version,updated_at=now() WHERE resources.user_id=EXCLUDED.user_id")
        .bind(r.id).bind(user).bind(&r.kind).bind(app.vault.seal(&data)?).bind(r.version).execute(conn).await?;
    Ok(())
}
pub async fn lock(conn: &mut PgConnection, user: Uuid) -> Result<(), Error> {
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(user)
        .fetch_one(conn)
        .await?;
    Ok(())
}
pub async fn notify(conn: &mut PgConnection, user: Uuid) -> Result<(), Error> {
    sqlx::query("SELECT pg_notify('camofy_changes',$1)")
        .bind(user.to_string())
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn rebuild(app: &App, conn: &mut PgConnection, user: Uuid) -> Result<(), Error> {
    rebuild_selected(app, conn, user, None).await
}
pub async fn rebuild_selected(
    app: &App,
    conn: &mut PgConnection,
    user: Uuid,
    ids: Option<&[Uuid]>,
) -> Result<(), Error> {
    let resources = list(app, conn, user).await?;
    for original in resources
        .iter()
        .filter(|r| r.kind == "bundle" && ids.is_none_or(|ids| ids.contains(&r.id)))
    {
        let mut bundle = original.clone();
        let result = render_bundle(&resources, &bundle.data, &app.origin);
        match result {
            Ok((artifacts, selections)) => {
                bundle.data["system_profile"] = system_profile(&app.origin)?;
                let content_hash = camofy::digest(serde_json::to_vec(&json!([
                    &artifacts,
                    &selections,
                    crate::catalog::lock_manifest(&resources, &bundle.data)
                ]))?);
                if bundle.data["published_hash"] == content_hash {
                    bundle.data["error"] = Value::Null;
                    put(app, conn, user, &bundle).await?;
                    continue;
                }
                let revision = Uuid::new_v4();
                sqlx::query("INSERT INTO revisions(id,user_id,bundle_id,artifacts,selections) VALUES($1,$2,$3,$4,$5)").bind(revision).bind(user).bind(bundle.id).bind(app.vault.seal(&artifacts)?).bind(selections).execute(&mut *conn).await?;
                sqlx::query("UPDATE revisions SET catalog_lock=$2 WHERE id=$1")
                    .bind(revision)
                    .bind(crate::catalog::lock_manifest(&resources, &bundle.data))
                    .execute(&mut *conn)
                    .await?;
                bundle.data["published_revision"] = json!(revision);
                bundle.data["published_hash"] = json!(content_hash);
                bundle.data["outputs"] = json!(
                    artifacts
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(k, v)| (k.clone(), json!({"error":v["error"]})))
                        .collect::<serde_json::Map<_, _>>()
                );
                bundle.data["error"] = Value::Null;
                // Keep a bounded history for rollback; current revision is always newest.
                sqlx::query("DELETE FROM revisions WHERE bundle_id=$1 AND id NOT IN (SELECT id FROM revisions WHERE bundle_id=$1 ORDER BY created_at DESC LIMIT 10)").bind(bundle.id).execute(&mut *conn).await?;
            }
            Err(e) => bundle.data["error"] = json!(e.to_string()),
        }
        put(app, conn, user, &bundle).await?;
    }
    notify(conn, user).await
}

/// Computed, immutable final profile: it is not a user-editable resource/binding.
pub fn system_profile(origin: &str) -> anyhow::Result<Value> {
    let content = serde_yaml::to_string(&camofy::engine::cloud_safety_profile(origin)?)?;
    Ok(json!({"name":"系统 · 云端直连保护","content":content,"locked":true,"version":1}))
}

pub fn render_bundle(
    resources: &[Resource],
    data: &Value,
    origin: &str,
) -> anyhow::Result<(Value, Value)> {
    let bindings = data["profiles"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("identity requires ordered profile bindings"))?;
    let mut profiles = Vec::new();
    for binding in bindings {
        if binding["enabled"] != true {
            continue;
        }
        let p = resources
            .iter()
            .find(|r| {
                r.kind == "profile"
                    && r.id.to_string() == binding["profile_id"].as_str().unwrap_or("")
            })
            .ok_or_else(|| anyhow::anyhow!("profile binding not found"))?;
        if p.data["store"].is_object() {
            profiles.push(crate::catalog::compile(p, binding)?);
        } else {
            profiles.push(
                p.data["content"]
                    .as_str()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "profile {} has no successfully fetched content",
                            p.data["name"]
                        )
                    })?
                    .to_string(),
            );
        }
    }
    anyhow::ensure!(
        !profiles.is_empty(),
        "enable at least one profile in this identity"
    );
    profiles.push(
        system_profile(origin)?["content"]
            .as_str()
            .unwrap()
            .to_owned(),
    );
    let selections = data.get("selections").cloned().unwrap_or(json!({}));
    let v =
        camofy::engine::compose_profiles(&profiles, &serde_json::from_value(selections.clone())?)?;
    let mut artifacts = serde_json::Map::new();
    for (format, result) in [
        ("clash", camofy::engine::mihomo(&v, false)),
        ("router", camofy::engine::mihomo(&v, true)),
        ("shadowrocket-nodes", camofy::engine::shadowrocket_nodes(&v)),
        ("shadowrocket", camofy::engine::shadowrocket_full(&v)),
    ] {
        artifacts.insert(
            format.into(),
            match result.and_then(|content| {
                let content = if format == "shadowrocket-nodes" {
                    content
                } else {
                    format!("{}{}", crate::catalog::notices(resources, data), content)
                };
                anyhow::ensure!(
                    content.len() <= 4 * 1024 * 1024,
                    "merged output exceeds 4 MiB"
                );
                Ok(content)
            }) {
                Ok(content) => json!({"hash":camofy::digest(content.as_bytes()),"content":content}),
                Err(e) => json!({"error":e.to_string()}),
            },
        );
    }
    for format in ["clash", "router"] {
        anyhow::ensure!(
            artifacts[format]["content"].is_string(),
            "{} output failed: {}",
            format,
            artifacts[format]["error"]
        );
    }
    Ok((Value::Object(artifacts), selections))
}
