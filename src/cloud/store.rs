use crate::{App, Error};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
pub struct Resource {
    pub id: Uuid,
    pub kind: String,
    pub version: i64,
    pub data: Value,
}

/// One-time data migration. This changes stored bindings, not published revisions;
/// the next successful compilation still controls when devices receive new YAML.
fn migrate_policy_data(kind: &str, data: &mut Value) -> anyhow::Result<bool> {
    let object = data
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("resource data must be an object"))?;
    let mut changed = false;
    if kind == "profile" {
        for field in ["inputs", "exports"] {
            if let Some(old) = object.remove(field) {
                anyhow::ensure!(
                    old.is_null() || old.as_array().is_some_and(Vec::is_empty),
                    "obsolete {field} declaration needs manual migration"
                );
                changed = true;
            }
        }
    }
    if kind != "bundle" {
        return Ok(changed);
    }
    if let Some(old) = object.remove("default_outbound") {
        anyhow::ensure!(
            old.is_null(),
            "obsolete default outbound needs manual migration"
        );
        changed = true;
    }
    for item in object
        .get_mut("profiles")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let fields = item
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("invalid Profile binding"))?;
        if let Some(old) = fields.remove("capability_bindings") {
            anyhow::ensure!(
                old.is_null() || old.as_object().is_some_and(serde_json::Map::is_empty),
                "obsolete outbound binding needs manual migration"
            );
            changed = true;
        }
        let Some(old) = fields.remove("parameters") else {
            continue;
        };
        changed = true;
        if old.is_null() {
            continue;
        }
        let params = old
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("invalid package parameters"))?;
        anyhow::ensure!(
            params.keys().all(|key| key == "policy"),
            "unknown package parameter needs manual migration"
        );
        let Some(policy) = params.get("policy") else {
            continue;
        };
        let policy = policy
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("invalid package policy"))?;
        let bindings = fields
            .entry("variable_bindings")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("invalid variable bindings"))?;
        let replacement = json!({"source":"literal","value":policy});
        if let Some(current) = bindings.get("policy") {
            anyhow::ensure!(current == &replacement, "package policy binding conflict");
        } else {
            bindings.insert("policy".into(), replacement);
        }
    }
    Ok(changed)
}

pub async fn migrate_policy_bindings(app: &App) -> anyhow::Result<usize> {
    const KEY: &str = "profile-variable-bindings-v1";
    let mut tx = app.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(835213, 1)")
        .execute(&mut *tx)
        .await?;
    // Keep the marker in an existing table so the previous binary can start
    // unchanged if this release needs to roll back.
    let done: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM admin_audit_events WHERE action=$1)")
            .bind(KEY)
            .fetch_one(&mut *tx)
            .await?;
    if done {
        tx.commit().await?;
        return Ok(0);
    }
    sqlx::query("LOCK TABLE resources IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    let rows = sqlx::query("SELECT id,kind,version,data FROM resources WHERE kind IN ('profile','bundle') ORDER BY id FOR UPDATE")
        .fetch_all(&mut *tx).await?;
    let mut migrated = 0;
    for row in rows {
        let id: Uuid = row.get("id");
        let kind: String = row.get("kind");
        let version: i64 = row.get("version");
        let mut data = app.vault.open(row.get("data"))?;
        if !migrate_policy_data(&kind, &mut data)? {
            continue;
        }
        let updated = sqlx::query("UPDATE resources SET data=$2,version=version+1,updated_at=now() WHERE id=$1 AND version=$3")
            .bind(id).bind(app.vault.seal(&data)?).bind(version).execute(&mut *tx).await?;
        anyhow::ensure!(
            updated.rows_affected() == 1,
            "concurrent resource edit during policy migration"
        );
        migrated += 1;
    }
    sqlx::query("INSERT INTO admin_audit_events(action) VALUES($1)")
        .bind(KEY)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(migrated)
}

fn old_export_reference(value: &Value) -> anyhow::Result<Option<(Uuid, String)>> {
    if value["source"] != "export" || value.get("profile_id").is_none() {
        return Ok(None);
    }
    let id: Uuid = value["profile_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("invalid old export provider ID"))?
        .parse()?;
    let key = value["key"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("invalid old export key"))?;
    Ok(Some((id, key.to_owned())))
}

fn identity_choices(data: &Value) -> Vec<&Value> {
    let mut out = Vec::new();
    for item in data["profiles"].as_array().into_iter().flatten() {
        out.extend(
            item["variable_bindings"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.values()),
        );
    }
    for item in data["identity_values"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
    {
        if let Some(binding) = item.get("binding") {
            out.push(binding);
        }
    }
    out
}

fn rewrite_old_export(
    value: &mut Value,
    aliases: &BTreeMap<(Uuid, String), String>,
) -> anyhow::Result<bool> {
    let Some(pair) = old_export_reference(value)? else {
        return Ok(false);
    };
    let Some(alias) = aliases.get(&pair) else {
        return Ok(false);
    };
    value["key"] = json!(alias);
    // Keep the old ID as inert data for a bounded image rollback. The new
    // resolver ignores it and all newly created bindings omit it.
    Ok(true)
}

/// Provider-specific bindings that would change meaning under key resolution
/// receive a unique export key. Non-ambiguous bindings already resolve by key
/// and retain their human-readable names. No provider-ID resolver branch remains.
fn migrate_export_data(resources: &mut [Resource]) -> anyhow::Result<BTreeSet<Uuid>> {
    let mut pairs = BTreeSet::new();
    let mut used = BTreeSet::new();
    for resource in resources.iter() {
        if resource.kind == "profile" {
            for item in resource.data["provides"].as_array().into_iter().flatten() {
                if let Some(key) = item["key"].as_str() {
                    used.insert(key.to_owned());
                }
            }
        }
    }
    for identity in resources.iter().filter(|r| r.kind == "bundle") {
        let enabled: BTreeSet<Uuid> = identity.data["profiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|item| item["enabled"] == true)
            .filter_map(|item| item["profile_id"].as_str().and_then(|id| id.parse().ok()))
            .collect();
        for choice in identity_choices(&identity.data) {
            let Some((provider, key)) = old_export_reference(choice)? else {
                continue;
            };
            let matching: Vec<Uuid> = resources
                .iter()
                .filter(|r| r.kind == "profile" && enabled.contains(&r.id))
                .filter(|r| {
                    r.data["provides"]
                        .as_array()
                        .is_some_and(|items| items.iter().any(|item| item["key"] == key))
                })
                .map(|r| r.id)
                .collect();
            if matching.len() != 1 || matching[0] != provider {
                pairs.insert((provider, key));
            }
        }
    }
    let mut aliases = BTreeMap::new();
    for (provider, key) in pairs {
        let base = format!(
            "bound_{}_{}",
            provider.simple(),
            &camofy::digest(&key)[..12]
        );
        let mut alias = base.clone();
        let mut suffix = 0;
        while used.contains(&alias) {
            suffix += 1;
            alias = format!("{base}_{suffix}");
        }
        anyhow::ensure!(alias.len() <= 64, "export alias too long");
        used.insert(alias.clone());
        aliases.insert((provider, key), alias);
    }
    if aliases.is_empty() {
        return Ok(BTreeSet::new());
    }
    let mut changed = BTreeSet::new();
    for resource in resources.iter_mut() {
        if resource.kind == "profile" {
            for item in resource
                .data
                .get_mut("provides")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                let Some(key) = item["key"].as_str() else {
                    continue;
                };
                if let Some(alias) = aliases.get(&(resource.id, key.to_owned())) {
                    item["key"] = json!(alias);
                    changed.insert(resource.id);
                }
            }
        } else if resource.kind == "bundle" {
            let mut modified = false;
            for item in resource
                .data
                .get_mut("profiles")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                for choice in item
                    .get_mut("variable_bindings")
                    .and_then(Value::as_object_mut)
                    .into_iter()
                    .flat_map(|m| m.values_mut())
                {
                    modified |= rewrite_old_export(choice, &aliases)?;
                }
            }
            for item in resource
                .data
                .get_mut("identity_values")
                .and_then(Value::as_object_mut)
                .into_iter()
                .flat_map(|m| m.values_mut())
            {
                if let Some(choice) = item.get_mut("binding") {
                    modified |= rewrite_old_export(choice, &aliases)?;
                }
            }
            if modified {
                changed.insert(resource.id);
            }
        }
    }
    Ok(changed)
}

pub async fn migrate_export_bindings(app: &App) -> anyhow::Result<usize> {
    const KEY: &str = "profile-export-variables-v2";
    let mut tx = app.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(835213, 1)")
        .execute(&mut *tx)
        .await?;
    let done: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM admin_audit_events WHERE action=$1)")
            .bind(KEY)
            .fetch_one(&mut *tx)
            .await?;
    if done {
        tx.commit().await?;
        return Ok(0);
    }
    sqlx::query("LOCK TABLE resources IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    let rows = sqlx::query("SELECT id,user_id,kind,version,data FROM resources WHERE kind IN ('profile','bundle') ORDER BY user_id,id FOR UPDATE")
        .fetch_all(&mut *tx).await?;
    let mut tenants: BTreeMap<Uuid, Vec<Resource>> = BTreeMap::new();
    for row in rows {
        let user: Uuid = row.get("user_id");
        tenants.entry(user).or_default().push(Resource {
            id: row.get("id"),
            kind: row.get("kind"),
            version: row.get("version"),
            data: app.vault.open(row.get("data"))?,
        });
    }
    let mut migrated = 0;
    for resources in tenants.values_mut() {
        let changed = migrate_export_data(resources)?;
        for resource in resources.iter().filter(|r| changed.contains(&r.id)) {
            let updated = sqlx::query("UPDATE resources SET data=$2,version=version+1,updated_at=now() WHERE id=$1 AND version=$3")
                .bind(resource.id).bind(app.vault.seal(&resource.data)?).bind(resource.version)
                .execute(&mut *tx).await?;
            anyhow::ensure!(
                updated.rows_affected() == 1,
                "concurrent resource edit during export migration"
            );
            migrated += 1;
        }
    }
    sqlx::query("INSERT INTO admin_audit_events(action) VALUES($1)")
        .bind(KEY)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(migrated)
}

#[cfg(test)]
mod export_migration_tests {
    use super::*;
    use crate::security::Vault;
    use crate::variables::Resolver;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use sqlx::postgres::PgPoolOptions;
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    fn profile(id: Uuid, name: &str, key: &str, value: &str) -> Resource {
        Resource {
            id,
            kind: "profile".into(),
            version: 1,
            data: json!({"name":name,"type":"source","content":"{}",
                "provides":[{"key":key,"label":"Group","type":"string",
                    "selector":{"source":"literal","value":value}}]}),
        }
    }

    fn consumer(id: Uuid) -> Resource {
        Resource {
            id,
            kind: "profile".into(),
            version: 1,
            data: json!({"name":"consumer","type":"overlay",
                "content":"first: '{{camofy.first}}'\nsecond: '{{camofy.second}}'",
                "variables":[{"key":"first","label":"First","type":"string","required":true},
                    {"key":"second","label":"Second","type":"string","required":true}]}),
        }
    }

    #[test]
    fn ambiguous_old_bindings_receive_unique_keys_without_changing_values() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let i = Uuid::new_v4();
        let mut resources = vec![
            profile(a, "a", "route", "A"),
            profile(b, "b", "route", "B"),
            consumer(c),
            Resource {
                id: i,
                kind: "bundle".into(),
                version: 1,
                data: json!({"published_revision":"unchanged","profiles":[
                    {"profile_id":a,"enabled":true}, {"profile_id":b,"enabled":true},
                    {"profile_id":c,"enabled":true,"variable_bindings":{
                        "first":{"source":"export","profile_id":a,"key":"route"},
                        "second":{"source":"export","profile_id":b,"key":"route"}}}]}),
            },
        ];
        let changed = migrate_export_data(&mut resources).unwrap();
        assert_eq!(changed, BTreeSet::from([a, b, i]));
        let first = resources[0].data["provides"][0]["key"].as_str().unwrap();
        let second = resources[1].data["provides"][0]["key"].as_str().unwrap();
        assert_ne!(first, second);
        assert_eq!(
            resources[3].data["profiles"][2]["variable_bindings"]["first"]["key"],
            first
        );
        assert_eq!(
            resources[3].data["profiles"][2]["variable_bindings"]["second"]["key"],
            second
        );
        assert_eq!(resources[3].data["published_revision"], "unchanged");
        let resolver = Resolver::new(&resources, &resources[3].data).unwrap();
        let rendered = resolver.compile(&resources[2]).unwrap();
        assert!(rendered.contains("first: A") && rendered.contains("second: B"));
    }

    #[test]
    fn safe_old_binding_needs_no_alias_and_can_follow_enabled_provider() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let mut resources = vec![
            profile(a, "a", "route", "A"),
            profile(b, "b", "route", "B"),
            consumer(c),
            Resource {
                id: Uuid::new_v4(),
                kind: "bundle".into(),
                version: 1,
                data: json!({"profiles":[{"profile_id":a,"enabled":true},
                    {"profile_id":b,"enabled":false},
                    {"profile_id":c,"enabled":true,"variable_bindings":{
                        "first":{"source":"export","profile_id":a,"key":"route"},
                        "second":{"source":"literal","value":"constant"}}}]}),
            },
        ];
        assert!(migrate_export_data(&mut resources).unwrap().is_empty());
        resources[3].data["profiles"][0]["enabled"] = json!(false);
        resources[3].data["profiles"][1]["enabled"] = json!(true);
        let resolver = Resolver::new(&resources, &resources[3].data).unwrap();
        assert!(
            resolver
                .compile(&resources[2])
                .unwrap()
                .contains("first: B")
        );
    }

    #[test]
    fn disabled_old_provider_cannot_silently_rebind_to_another_export() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let mut resources = vec![
            profile(a, "a", "route", "A"),
            profile(b, "b", "route", "B"),
            consumer(c),
            Resource {
                id: Uuid::new_v4(),
                kind: "bundle".into(),
                version: 1,
                data: json!({"profiles":[{"profile_id":a,"enabled":false},
                    {"profile_id":b,"enabled":true},
                    {"profile_id":c,"enabled":true,"variable_bindings":{
                        "first":{"source":"export","profile_id":a,"key":"route"},
                        "second":{"source":"literal","value":"constant"}}}]}),
            },
        ];
        let changed = migrate_export_data(&mut resources).unwrap();
        assert_eq!(changed, BTreeSet::from([a, resources[3].id]));
        let resolver = Resolver::new(&resources, &resources[3].data).unwrap();
        assert!(
            resolver
                .compile(&resources[2])
                .unwrap_err()
                .to_string()
                .contains("no enabled")
        );
    }

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL pointing to a disposable PostgreSQL"]
    async fn encrypted_export_migration_is_atomic_idempotent_and_keeps_revisions() {
        let db = PgPoolOptions::new()
            .max_connections(3)
            .connect(&std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required"))
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let app = App {
            db: db.clone(),
            vault: Vault::new(&STANDARD.encode([47; 32])).unwrap(),
            origin: "https://cloud.example".into(),
            legacy_origins: vec![],
            secure: false,
            registration: false,
            private_egress: false,
            workers: 1,
            captcha: None,
            topics: Default::default(),
            hash_slots: Arc::new(Semaphore::new(1)),
        };
        let user = Uuid::new_v4();
        sqlx::query("INSERT INTO users(id,email,password) VALUES($1,$2,'unused')")
            .bind(user)
            .bind(format!("{user}@example.test"))
            .execute(&db)
            .await
            .unwrap();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let identity = Uuid::new_v4();
        let invalid = Uuid::new_v4();
        let data = [
            (first, "profile", profile(first, "first", "route", "A").data),
            (
                second,
                "profile",
                profile(second, "second", "route", "B").data,
            ),
            (
                identity,
                "bundle",
                json!({"name":"identity","published_revision":"keep-revision",
                "profiles":[{"profile_id":first,"enabled":true},{"profile_id":second,"enabled":true,
                    "variable_bindings":{"route":{"source":"export","profile_id":first,"key":"route"}}}]}),
            ),
            (
                invalid,
                "bundle",
                json!({"name":"invalid","profiles":[{"profile_id":second,"enabled":true,
                "variable_bindings":{"route":{"source":"export","profile_id":"not-a-uuid","key":"route"}}}]}),
            ),
        ];
        for (id, kind, data) in &data {
            sqlx::query(
                "INSERT INTO resources(id,user_id,kind,data,version) VALUES($1,$2,$3,$4,1)",
            )
            .bind(id)
            .bind(user)
            .bind(kind)
            .bind(app.vault.seal(data).unwrap())
            .execute(&db)
            .await
            .unwrap();
        }
        assert!(migrate_export_bindings(&app).await.is_err());
        let unchanged = sqlx::query("SELECT data,version FROM resources WHERE id=$1")
            .bind(first)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(unchanged.get::<i64, _>("version"), 1);
        assert_eq!(app.vault.open(unchanged.get("data")).unwrap(), data[0].2);
        sqlx::query("DELETE FROM resources WHERE id=$1")
            .bind(invalid)
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(migrate_export_bindings(&app).await.unwrap(), 2);
        let source = sqlx::query("SELECT data,version FROM resources WHERE id=$1")
            .bind(first)
            .fetch_one(&db)
            .await
            .unwrap();
        let new_key = app.vault.open(source.get("data")).unwrap()["provides"][0]["key"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(new_key, "route");
        assert_eq!(source.get::<i64, _>("version"), 2);
        let saved_identity: Value = sqlx::query_scalar("SELECT data FROM resources WHERE id=$1")
            .bind(identity)
            .fetch_one(&db)
            .await
            .unwrap();
        let saved_identity = app.vault.open(saved_identity).unwrap();
        assert_eq!(
            saved_identity["profiles"][1]["variable_bindings"]["route"]["key"],
            new_key
        );
        assert_eq!(saved_identity["published_revision"], "keep-revision");
        assert_eq!(migrate_export_bindings(&app).await.unwrap(), 0);
        sqlx::query("DELETE FROM users WHERE id=$1")
            .bind(user)
            .execute(&db)
            .await
            .unwrap();
    }
}

#[cfg(test)]
mod policy_migration_tests {
    use super::*;
    use crate::security::Vault;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use sqlx::postgres::PgPoolOptions;
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    #[test]
    fn package_policy_moves_to_the_only_binding_model() {
        let mut data = json!({"name":"identity","published_revision":"unchanged",
            "profiles":[{"profile_id":Uuid::new_v4(),"enabled":true,
                "parameters":{"policy":"DIRECT"}}]});
        assert!(migrate_policy_data("bundle", &mut data).unwrap());
        assert_eq!(
            data["profiles"][0]["variable_bindings"]["policy"],
            json!({"source":"literal","value":"DIRECT"})
        );
        assert!(data["profiles"][0].get("parameters").is_none());
        assert_eq!(data["published_revision"], "unchanged");
        assert!(!migrate_policy_data("bundle", &mut data).unwrap());
    }

    #[test]
    fn unknown_obsolete_bindings_abort_without_persisting_partial_changes() {
        let mut data = json!({"profiles":[{"profile_id":Uuid::new_v4(),"enabled":true,
            "parameters":{"policy":"DIRECT"},
            "capability_bindings":{"hop":{"source":"literal","value":"A"}}}]});
        assert!(migrate_policy_data("bundle", &mut data).is_err());
    }

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL pointing to a disposable PostgreSQL"]
    async fn encrypted_policy_migration_is_atomic_idempotent_and_keeps_revisions() {
        let db = PgPoolOptions::new()
            .max_connections(3)
            .connect(&std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required"))
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let app = App {
            db: db.clone(),
            vault: Vault::new(&STANDARD.encode([42; 32])).unwrap(),
            origin: "https://cloud.example".into(),
            legacy_origins: vec![],
            secure: false,
            registration: false,
            private_egress: false,
            workers: 1,
            captcha: None,
            topics: Default::default(),
            hash_slots: Arc::new(Semaphore::new(1)),
        };
        let user = Uuid::new_v4();
        sqlx::query("INSERT INTO users(id,email,password) VALUES($1,$2,'unused')")
            .bind(user)
            .bind(format!("{user}@example.test"))
            .execute(&db)
            .await
            .unwrap();
        let base = Uuid::new_v4().as_u128();
        let good = Uuid::from_u128(base & !1);
        let bad = Uuid::from_u128(base | 1);
        let original = json!({"name":"identity","published_revision":"do-not-rebuild",
            "profiles":[{"profile_id":Uuid::new_v4(),"enabled":true,
                "parameters":{"policy":"DIRECT"}}]});
        let invalid = json!({"name":"other","profiles":[{"profile_id":Uuid::new_v4(),
            "capability_bindings":{"unknown":{"source":"literal","value":"X"}}}]});
        for (id, data) in [(good, &original), (bad, &invalid)] {
            sqlx::query(
                "INSERT INTO resources(id,user_id,kind,data,version) VALUES($1,$2,'bundle',$3,1)",
            )
            .bind(id)
            .bind(user)
            .bind(app.vault.seal(data).unwrap())
            .execute(&db)
            .await
            .unwrap();
        }
        assert!(migrate_policy_bindings(&app).await.is_err());
        let marked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM admin_audit_events WHERE action='profile-variable-bindings-v1')",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert!(!marked);
        let row = sqlx::query("SELECT data,version FROM resources WHERE id=$1")
            .bind(good)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(row.get::<i64, _>("version"), 1);
        assert_eq!(app.vault.open(row.get("data")).unwrap(), original);
        sqlx::query("DELETE FROM resources WHERE id=$1")
            .bind(bad)
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(migrate_policy_bindings(&app).await.unwrap(), 1);
        let row = sqlx::query("SELECT data,version FROM resources WHERE id=$1")
            .bind(good)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(row.get::<i64, _>("version"), 2);
        let migrated = app.vault.open(row.get("data")).unwrap();
        assert_eq!(migrated["published_revision"], "do-not-rebuild");
        assert_eq!(
            migrated["profiles"][0]["variable_bindings"]["policy"]["value"],
            "DIRECT"
        );
        let marked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM admin_audit_events WHERE action='profile-variable-bindings-v1')",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert!(marked);
        assert_eq!(migrate_policy_bindings(&app).await.unwrap(), 0);
        sqlx::query("DELETE FROM users WHERE id=$1")
            .bind(user)
            .execute(&db)
            .await
            .unwrap();
    }
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
                let variable_lock = if crate::variables::active(&effective, &bundle.data) {
                    crate::variables::Resolver::new(&effective, &bundle.data)?.lock()?
                } else {
                    json!([])
                };
                let filtered = !source_filter_lock.as_array().is_some_and(Vec::is_empty);
                let capable = !variable_lock.as_array().is_some_and(Vec::is_empty);
                let hash_input = if filtered && capable {
                    json!([
                        &artifacts,
                        &selections,
                        &catalog_lock,
                        &variable_lock,
                        &source_filter_lock
                    ])
                } else if filtered {
                    json!([&artifacts, &selections, &catalog_lock, &source_filter_lock])
                } else if capable {
                    json!([&artifacts, &selections, &catalog_lock, &variable_lock])
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
                sqlx::query("INSERT INTO revisions(id,user_id,bundle_id,artifacts,selections,usage_sources,catalog_lock,capability_lock,source_filter_lock) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)").bind(revision).bind(user).bind(bundle.id).bind(app.vault.seal(&artifacts)?).bind(selections).bind(usage_sources).bind(catalog_lock).bind(variable_lock).bind(source_filter_lock).execute(&mut *conn).await?;
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
        if source_filter(binding)?.is_none() {
            continue;
        }
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
    let node_policy = camofy::node_filter::Policy::parse(&data["node_filter"])?;
    let manual = camofy::node_filter::apply(
        &base,
        &camofy::node_filter::Policy {
            auto: false,
            exclude_types: node_policy.exclude_types.clone(),
        },
        Default::default(),
    )?;
    let base = manual.config;
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
    // This metadata stays inside the encrypted revision. It freezes the policy
    // and input used for all request-specific views without changing agent hashes.
    artifacts.get_mut("router").unwrap()["compatibility"] = json!({
        "policy":node_policy,"base":serde_yaml::to_string(&v)?,
        "notices":crate::catalog::notices(resources,data),"manual_report":manual.report,
        "matrix_version":camofy::compatibility::registry().version
    });
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
