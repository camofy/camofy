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
    for r in &mut records {
        if r.kind == "profile" {
            r.data["_exports"] = crate::capabilities::available_exports(&r.data);
            r.data["_candidates"] = crate::variables::candidates(&r.data);
        }
    }
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
    if record.kind == "profile" {
        record.data["_exports"] = crate::capabilities::available_exports(&record.data);
        record.data["_candidates"] = crate::variables::candidates(&record.data);
    }
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
    data.as_object_mut().unwrap().remove("_exports");
    data.as_object_mut().unwrap().remove("_candidates");
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
    let mut changed = false;
    for original in resources
        .iter()
        .filter(|r| r.kind == "bundle" && ids.is_none_or(|ids| ids.contains(&r.id)))
    {
        let mut bundle = original.clone();
        let result = render_bundle(&resources, &bundle.data, &app.origin);
        match result {
            Ok((artifacts, selections)) => {
                let usage_sources =
                    json!(crate::usage::sources(app, user, &resources, &bundle.data));
                bundle.data["system_profile"] = system_profile(&app.origin)?;
                let catalog_lock = crate::catalog::lock_manifest(&resources, &bundle.data);
                let source_filter_lock = source_filter_lock(&resources, &bundle.data)?;
                let effective = effective_resources(&resources, &bundle.data)?;
                let legacy_lock = crate::capabilities::dependency_lock(&effective, &bundle.data)?;
                let variable_lock = if crate::variables::active(&effective, &bundle.data) {
                    crate::variables::Resolver::new(&effective, &bundle.data)?.lock()?
                } else {
                    json!([])
                };
                let capability_lock = if variable_lock.as_array().is_some_and(Vec::is_empty) {
                    legacy_lock
                } else {
                    json!({"legacy":legacy_lock,"variables":variable_lock})
                };
                // Preserve existing fingerprints when no source filter is active.
                let filtered = !source_filter_lock.as_array().is_some_and(Vec::is_empty);
                let capable = !capability_lock.as_array().is_some_and(Vec::is_empty);
                let hash_input = if filtered && capable {
                    json!([
                        &artifacts,
                        &selections,
                        &catalog_lock,
                        &capability_lock,
                        &source_filter_lock
                    ])
                } else if filtered {
                    json!([&artifacts, &selections, &catalog_lock, &source_filter_lock])
                } else if capable {
                    json!([&artifacts, &selections, &catalog_lock, &capability_lock])
                } else if catalog_lock.as_array().is_some_and(Vec::is_empty) {
                    json!([&artifacts, &selections])
                } else {
                    json!([&artifacts, &selections, &catalog_lock])
                };
                let content_hash = camofy::digest(serde_json::to_vec(&hash_input)?);
                let current = bundle.data["published_revision"]
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok());
                let previous: Option<Value> = sqlx::query_scalar(
                    "SELECT usage_sources FROM revisions WHERE id=$1 AND user_id=$2",
                )
                .bind(current)
                .bind(user)
                .fetch_optional(&mut *conn)
                .await?
                .flatten();
                if bundle.data["published_hash"] == content_hash
                    && (previous.is_none() || previous.as_ref() == Some(&usage_sources))
                {
                    if previous.is_none() {
                        sqlx::query(
                            "UPDATE revisions SET usage_sources=$3 WHERE id=$1 AND user_id=$2",
                        )
                        .bind(current)
                        .bind(user)
                        .bind(&usage_sources)
                        .execute(&mut *conn)
                        .await?;
                    }
                    bundle.data["error"] = Value::Null;
                    put(app, conn, user, &bundle).await?;
                    continue;
                }
                let revision = Uuid::new_v4();
                sqlx::query("INSERT INTO revisions(id,user_id,bundle_id,artifacts,selections,usage_sources,catalog_lock,capability_lock,source_filter_lock) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)").bind(revision).bind(user).bind(bundle.id).bind(app.vault.seal(&artifacts)?).bind(selections).bind(usage_sources).bind(catalog_lock).bind(capability_lock).bind(source_filter_lock).execute(&mut *conn).await?;
                changed = true;
                tracing::info!(bundle_id = %bundle.id, %revision,
                    "bundle configuration revision staged");
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
            Err(e) => {
                tracing::warn!(bundle_id = %bundle.id,
                    bindings = bundle.data["profiles"].as_array().map_or(0, Vec::len),
                    "bundle configuration render failed; previous revision retained");
                bundle.data["error"] = json!(e.to_string());
            }
        }
        put(app, conn, user, &bundle).await?;
    }
    if changed {
        notify(conn, user).await?;
    }
    Ok(())
}

/// Computed, immutable final profile: it is not a user-editable resource/binding.
pub fn system_profile(origin: &str) -> anyhow::Result<Value> {
    let content = serde_yaml::to_string(&camofy::engine::cloud_safety_profile(origin)?)?;
    Ok(json!({"name":"系统 · 云端直连保护","content":content,"locked":true,"version":1}))
}

/// No filter (including an explicit empty include list) preserves legacy full-source behavior.
/// The two flags represent nodes and groups; additional categories require an explicit API change.
pub fn source_filter(binding: &Value) -> anyhow::Result<Option<(bool, bool)>> {
    let Some(filter) = binding.get("source_filter") else {
        return Ok(None);
    };
    if filter.is_null() {
        return Ok(None);
    }
    let object = filter
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("source_filter must contain an include list"))?;
    anyhow::ensure!(
        object.len() == 1 && object.contains_key("include"),
        "source_filter only supports include"
    );
    let fields = object["include"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("source_filter.include must be a list"))?;
    anyhow::ensure!(fields.len() <= 2, "source_filter has too many fields");
    let mut nodes = false;
    let mut groups = false;
    for field in fields {
        match field.as_str() {
            Some("proxies") if !nodes => nodes = true,
            Some("proxy-groups") if !groups => groups = true,
            _ => anyhow::bail!("source_filter contains an unknown or duplicate field"),
        }
    }
    Ok((nodes || groups).then_some((nodes, groups)))
}

fn filtered_source_content(content: &str, binding: &Value) -> anyhow::Result<String> {
    let Some((nodes, groups)) = source_filter(binding)? else {
        return Ok(content.to_owned());
    };
    let parsed = camofy::engine::parse(content)?;
    let mut selected = serde_yaml::Mapping::new();
    for (key, value) in parsed.as_mapping().unwrap() {
        let keep = match key.as_str() {
            Some("proxies" | "prepend-proxies" | "append-proxies" | "proxy-providers") => nodes,
            Some(
                "proxy-groups"
                | "prepend-proxy-groups"
                | "append-proxy-groups"
                | "proxy-group-patches",
            ) => groups,
            _ => false,
        };
        if keep {
            selected.insert(key.clone(), value.clone());
        }
    }
    Ok(serde_yaml::to_string(&selected)?)
}

/// Capabilities and YAML composition must see the same per-identity source snapshot.
/// Never mutate the stored upstream content shared by other identities.
pub fn effective_resources(resources: &[Resource], data: &Value) -> anyhow::Result<Vec<Resource>> {
    let mut effective = resources.to_vec();
    for binding in data["profiles"].as_array().into_iter().flatten() {
        if binding["enabled"] != true {
            continue;
        }
        let Some((nodes, groups)) = source_filter(binding)? else {
            continue;
        };
        let Some(profile) = effective.iter_mut().find(|r| {
            r.kind == "profile"
                && r.data["type"] == "source"
                && r.id.to_string() == binding["profile_id"].as_str().unwrap_or("")
        }) else {
            continue;
        };
        let content = profile.data["content"].as_str().ok_or_else(|| {
            anyhow::anyhow!(
                "profile {} has no successfully fetched content",
                profile.data["name"]
            )
        })?;
        profile.data["content"] = json!(filtered_source_content(content, binding)?);
        if let Some(exports) = profile.data["exports"].as_array_mut() {
            exports.retain(|entry| match entry["kind"].as_str() {
                Some("proxy") => nodes,
                Some("group") => groups,
                _ => true,
            });
        }
        profile.data["_exports"] = crate::capabilities::available_exports(&profile.data);
    }
    Ok(effective)
}

fn source_filter_lock(resources: &[Resource], data: &Value) -> anyhow::Result<Value> {
    let mut result = Vec::new();
    for binding in data["profiles"].as_array().into_iter().flatten() {
        if binding["enabled"] != true {
            continue;
        }
        let Some(profile) = resources.iter().find(|r| {
            r.kind == "profile" && r.id.to_string() == binding["profile_id"].as_str().unwrap_or("")
        }) else {
            continue;
        };
        if profile.data["type"] != "source" {
            continue;
        }
        if let Some((nodes, groups)) = source_filter(binding)? {
            let mut include = Vec::new();
            if nodes {
                include.push("proxies");
            }
            if groups {
                include.push("proxy-groups");
            }
            result.push(json!({"profile_id":profile.id,"include":include}));
        }
    }
    Ok(json!(result))
}

/// Explain collisions before name-based merging replaces an earlier source's node/group.
pub fn source_merge_warnings(resources: &[Resource], data: &Value) -> anyhow::Result<Vec<String>> {
    let mut seen = std::collections::BTreeMap::<(String, String), (Uuid, String)>::new();
    let mut warnings = Vec::new();
    let mut full_sources = 0;
    for binding in data["profiles"].as_array().into_iter().flatten() {
        if binding["enabled"] != true {
            continue;
        }
        let Some(profile) = resources.iter().find(|r| {
            r.kind == "profile"
                && r.id.to_string() == binding["profile_id"].as_str().unwrap_or("")
                && r.data["type"] == "source"
        }) else {
            continue;
        };
        if source_filter(binding)?.is_none() {
            full_sources += 1;
        }
        let Some(content) = profile.data["content"].as_str() else {
            continue;
        };
        let parsed = camofy::engine::parse(&filtered_source_content(content, binding)?)?;
        for (field, keys) in [
            ("代理", ["proxies", "prepend-proxies", "append-proxies"]),
            (
                "代理组",
                [
                    "proxy-groups",
                    "prepend-proxy-groups",
                    "append-proxy-groups",
                ],
            ),
        ] {
            for key in keys {
                for item in parsed[key].as_sequence().into_iter().flatten() {
                    let Some(name) = item["name"].as_str() else {
                        continue;
                    };
                    let current = profile.data["name"].as_str().unwrap_or("订阅源");
                    if let Some((previous_id, previous)) =
                        seen.insert((field.into(), name.into()), (profile.id, current.into()))
                        && previous_id != profile.id
                        && warnings.len() < 20
                    {
                        warnings.push(format!(
                            "{field}「{name}」在订阅源「{previous}」和「{current}」中同名；后者按现有合并顺序覆盖前者。"
                        ));
                    }
                }
            }
        }
    }
    if full_sources > 1 {
        warnings.insert(
            0,
            "多个订阅源提供完整配置；普通设置按关联顺序由后者覆盖。".into(),
        );
    }
    Ok(warnings)
}

pub fn render_bundle(
    resources: &[Resource],
    data: &Value,
    origin: &str,
) -> anyhow::Result<(Value, Value)> {
    let effective = effective_resources(resources, data)?;
    // Validate every declared reference, including an unused explicit default,
    // before considering this revision publishable.
    crate::capabilities::dependency_lock(&effective, data)?;
    let variable_resolver = crate::variables::active(&effective, data)
        .then(|| crate::variables::Resolver::new(&effective, data))
        .transpose()?;
    if let Some(resolver) = &variable_resolver {
        resolver.lock()?;
    }
    let bindings = data["profiles"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("identity requires ordered profile bindings"))?;
    let mut profiles = Vec::new();
    let mut rendered_profiles = Vec::new();
    let resolver = crate::capabilities::active_contracts(&effective, data)
        .then(|| crate::capabilities::Resolver::new(&effective, data))
        .transpose()?;
    for binding in bindings {
        if binding["enabled"] != true {
            continue;
        }
        let p = effective
            .iter()
            .find(|r| {
                r.kind == "profile"
                    && r.id.to_string() == binding["profile_id"].as_str().unwrap_or("")
            })
            .ok_or_else(|| anyhow::anyhow!("profile binding not found"))?;
        let content = if let Some(resolver) = &variable_resolver {
            resolver.compile(p)?
        } else if p.data["store"].is_object() {
            let mut resolved = binding.clone();
            if let Some(choice) = binding["capability_bindings"].get("policy") {
                let (policy, _) = resolver.as_ref().unwrap().resolve(Some(choice))?;
                resolved["parameters"]["policy"] = json!(policy);
            }
            crate::catalog::compile(p, &resolved)?
        } else {
            if let Some(resolver) = &resolver {
                resolver.compile(p, binding)?.0
            } else {
                p.data["content"]
                    .as_str()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "profile {} has no successfully fetched content",
                            p.data["name"]
                        )
                    })?
                    .to_string()
            }
        };
        rendered_profiles.push((p.id, content.clone()));
        profiles.push(content);
    }
    anyhow::ensure!(
        !profiles.is_empty(),
        "enable at least one profile in this identity"
    );
    if let Some(resolver) = &variable_resolver {
        resolver.validate_outbounds(&rendered_profiles)?;
    }
    profiles.push(
        system_profile(origin)?["content"]
            .as_str()
            .unwrap()
            .to_owned(),
    );
    let selections = data.get("selections").cloned().unwrap_or(json!({}));
    let base = camofy::engine::compose_profiles(&profiles, &Default::default())?;
    let mut v = base.clone();
    camofy::engine::selection_defaults(&mut v, &serde_json::from_value(selections.clone())?);
    let mut artifacts = serde_json::Map::new();
    for (format, result) in [
        ("agent", camofy::engine::mihomo(&base, true)),
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
