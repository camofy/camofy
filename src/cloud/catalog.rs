//! Reviewed declarative rule packages. No remote fetch, evaluation or caller-owned code.
use crate::{
    App, Error, auth,
    store::{self, Resource},
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub kind: String,
    pub value: String,
    #[serde(default)]
    pub no_resolve: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub url: String,
    pub revision: String,
    pub license: String,
    pub attribution: String,
    pub sha256: String,
    pub license_text: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub summary: String,
    pub category: String,
    pub notes: String,
    pub default_policy: Option<String>,
    pub sources: Vec<Source>,
}
fn bad(e: impl ToString) -> Error {
    Error::bad(e.to_string())
}
fn text_ok(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
fn hex(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn validate_rules(rules: &[Rule]) -> anyhow::Result<()> {
    anyhow::ensure!(
        !rules.is_empty() && rules.len() <= 10000,
        "a package needs 1–10000 rules"
    );
    let mut seen = HashSet::new();
    for r in rules {
        anyhow::ensure!(
            seen.insert((&r.kind, &r.value, r.no_resolve)),
            "duplicate package rule"
        );
        match r.kind.as_str() {
            "DOMAIN" | "DOMAIN-SUFFIX" => {
                anyhow::ensure!(
                    !r.no_resolve
                        && r.value.len() <= 253
                        && r.value.contains('.')
                        && r.value.split('.').all(|p| !p.is_empty()
                            && p.len() <= 63
                            && !p.starts_with('-')
                            && !p.ends_with('-')
                            && p.bytes().all(|b| b.is_ascii_lowercase()
                                || b.is_ascii_digit()
                                || b == b'-')),
                    "invalid domain rule: {}",
                    r.value
                );
            }
            "IP-CIDR" | "IP-CIDR6" => {
                let (ip, bits) = r
                    .value
                    .split_once('/')
                    .ok_or_else(|| anyhow::anyhow!("CIDR prefix required"))?;
                let ip: std::net::IpAddr = ip.parse()?;
                let bits: u8 = bits.parse()?;
                anyhow::ensure!(
                    if r.kind == "IP-CIDR" {
                        ip.is_ipv4() && bits <= 32
                    } else {
                        ip.is_ipv6() && bits <= 128
                    },
                    "invalid CIDR family/prefix"
                );
            }
            _ => anyhow::bail!("unsupported package rule type: {}", r.kind),
        }
    }
    anyhow::ensure!(
        serde_json::to_vec(rules)?.len() <= 1024 * 1024,
        "package exceeds 1 MiB"
    );
    Ok(())
}
fn validate_manifest(m: &Manifest) -> Result<(), Error> {
    if !text_ok(&m.name, 120)
        || !text_ok(&m.summary, 600)
        || !text_ok(&m.category, 40)
        || !text_ok(&m.notes, 3000)
        || m.sources.is_empty()
        || m.sources.len() > 16
    {
        return Err(bad("invalid package metadata"));
    }
    if m.default_policy
        .as_ref()
        .is_some_and(|s| !["DIRECT", "REJECT"].contains(&s.as_str()))
    {
        return Err(bad(
            "default policy must be DIRECT, REJECT or null (explicit selection)",
        ));
    }
    for s in &m.sources {
        let u = url::Url::parse(&s.url).map_err(bad)?;
        if u.scheme() != "https"
            || u.host_str().is_none()
            || !u.username().is_empty()
            || u.password().is_some()
            || u.query().is_some()
            || u.fragment().is_some()
            || !hex(&s.revision, 40)
            || !hex(&s.sha256, 64)
            || !text_ok(&s.license, 100)
            || !text_ok(&s.attribution, 1000)
            || s.license_text.trim().is_empty()
            || s.license_text.len() > 32000
            || s.license_text
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
        {
            return Err(bad(
                "source needs HTTPS provenance, commit, SHA256, license and attribution",
            ));
        }
    }
    Ok(())
}
pub async fn hydrate(conn: &mut PgConnection, resources: &mut [Resource]) -> Result<(), Error> {
    let ids: Vec<Uuid> = resources
        .iter()
        .filter_map(|r| r.data["store"]["version_id"].as_str()?.parse().ok())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let rows=sqlx::query("SELECT v.id,v.package_slug,v.version,v.manifest,a.hash,a.content,p.publisher FROM catalog_versions v JOIN catalog_packages p ON p.slug=v.package_slug JOIN catalog_version_artifacts x ON x.version_id=v.id AND x.role='rules' JOIN catalog_artifacts a ON a.hash=x.artifact_hash WHERE v.id=ANY($1)").bind(&ids).fetch_all(conn).await?;
    for r in resources {
        r.data.as_object_mut().unwrap().remove("_package");
        if let Some(id) = r.data["store"]["version_id"].as_str() {
            if let Some(v) = rows
                .iter()
                .find(|v| v.get::<Uuid, _>("id").to_string() == id)
            {
                r.data["_package"] = json!({"id":id,"slug":v.get::<String,_>("package_slug"),"version":v.get::<String,_>("version"),"manifest":v.get::<Value,_>("manifest"),"hash":v.get::<String,_>("hash"),"rules":v.get::<Value,_>("content"),"publisher":v.get::<String,_>("publisher")});
            } else {
                return Err(bad("installed package version unavailable"));
            }
        }
    }
    Ok(())
}
pub fn compile(r: &Resource, binding: &Value) -> anyhow::Result<String> {
    let p = &r.data["_package"];
    let m: Manifest = serde_json::from_value(p["manifest"].clone())?;
    let rules: Vec<Rule> = serde_json::from_value(p["rules"].clone())?;
    validate_rules(&rules)?;
    if let Some(params) = binding.get("parameters") {
        anyhow::ensure!(
            params
                .as_object()
                .is_some_and(|m| m.keys().all(|k| k == "policy")),
            "unsupported package parameters"
        );
        if let Some(v) = params.get("policy") {
            anyhow::ensure!(v.is_string(), "policy must be a string");
        }
    }
    let policy = binding["parameters"]["policy"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or(m.default_policy.as_deref())
        .ok_or_else(|| anyhow::anyhow!("{}: select a policy in this identity", m.name))?;
    anyhow::ensure!(
        text_ok(policy, 200) && !policy.contains(','),
        "invalid policy reference"
    );
    let rules: Vec<String> = rules
        .iter()
        .map(|r| {
            format!(
                "{},{},{}{}",
                r.kind,
                r.value,
                policy,
                if r.no_resolve { ",no-resolve" } else { "" }
            )
        })
        .collect();
    Ok(serde_yaml::to_string(&json!({"prepend-rules":rules}))?)
}
pub fn lock_manifest(resources: &[Resource], data: &Value) -> Value {
    json!(data["profiles"].as_array().into_iter().flatten().filter_map(|b| {
        let p=resources.iter().find(|r| Some(r.id.to_string()).as_deref()==b["profile_id"].as_str())?;
        if !p.data["store"].is_object() {return None;}
        Some(json!({"profile_id":p.id,"enabled":b["enabled"],"parameters":b["parameters"],"version_id":p.data["store"]["version_id"],"artifact_hash":p.data["_package"]["hash"]}))
    }).collect::<Vec<_>>())
}
/// Carry notices into redistributed rule-bearing configurations, not just the store UI.
pub fn notices(resources: &[Resource], data: &Value) -> String {
    let mut out = String::new();
    let mut seen = HashSet::new();
    for b in data["profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["enabled"] == true)
    {
        let Some(p) = resources
            .iter()
            .find(|p| Some(p.id.to_string()).as_deref() == b["profile_id"].as_str())
        else {
            continue;
        };
        let sources = p.data["_package"]["manifest"]["sources"]
            .as_array()
            .or_else(|| p.data["provenance"].as_array());
        for s in sources.into_iter().flatten() {
            // Provenance is metadata, not YAML. Fork metadata is tenant-editable;
            // prefix every YAML line separator, including standalone CR and NEL.
            if !seen.insert(s.to_string()) {
                continue;
            }
            for key in ["attribution", "url", "revision", "license", "license_text"] {
                if let Some(text) = s[key].as_str() {
                    for line in text.split(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}']) {
                        out.push_str("# ");
                        out.extend(
                            line.chars()
                                .map(|c| if c.is_control() && c != '\t' { ' ' } else { c }),
                        );
                        out.push('\n');
                    }
                }
            }
        }
    }
    out
}
pub fn diagnostics(content: &str) -> Vec<String> {
    let Ok(v) = camofy::engine::parse(content) else {
        return vec![];
    };
    let mut warnings = Vec::new();
    let mut matches: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut catchall = false;
    for r in v["rules"]
        .as_sequence()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
    {
        if warnings.len() >= 30 {
            break;
        }
        if catchall {
            warnings.push("存在位于 MATCH 之后的规则，不会被匹配；请检查最终顺序。".into());
            break;
        }
        let p: Vec<_> = r.split(',').collect();
        if p[0] == "MATCH" {
            catchall = true;
        }
        if p.len() >= 3 && ["DOMAIN", "DOMAIN-SUFFIX", "IP-CIDR", "IP-CIDR6"].contains(&p[0]) {
            if p[0] == "DOMAIN" || p[0] == "DOMAIN-SUFFIX" {
                let mut suffix = p[1];
                loop {
                    if let Some(first) = matches.get(&("DOMAIN-SUFFIX".into(), suffix.into())) {
                        if first != p[2] {
                            warnings.push(format!(
                                "{}：被前面的后缀 {} → {} 覆盖，后续 {} 不生效",
                                p[1], suffix, first, p[2]
                            ));
                        }
                        break;
                    }
                    let Some((_, parent)) = suffix.split_once('.') else {
                        break;
                    };
                    suffix = parent;
                }
            }
            let key = (p[0].to_string(), p[1].to_string());
            if let Some(first) = matches.get(&key) {
                if first != p[2] {
                    warnings.push(format!(
                        "{}：优先命中 {}，后续 {} 不生效",
                        p[1], first, p[2]
                    ));
                }
            } else {
                matches.insert(key, p[2].to_string());
            }
        }
    }
    warnings
}

pub async fn listing(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>, Error> {
    auth::user(&app, &h, false).await?;
    let rows=sqlx::query("SELECT DISTINCT ON(p.slug) p.slug,p.publisher,v.id,v.version,v.manifest FROM catalog_packages p JOIN catalog_versions v ON v.package_slug=p.slug ORDER BY p.slug,v.created_at DESC,v.id DESC LIMIT 100").fetch_all(&app.db).await?;
    Ok(Json(json!(rows.iter().map(|r|json!({"slug":r.get::<String,_>("slug"),"publisher":r.get::<String,_>("publisher"),"id":r.get::<Uuid,_>("id"),"version":r.get::<String,_>("version"),"manifest":r.get::<Value,_>("manifest")})).collect::<Vec<_>>())))
}
pub async fn detail(
    State(app): State<App>,
    h: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, false).await?;
    let rows=sqlx::query("SELECT p.publisher,p.owner_id,v.id,v.version,v.manifest,a.content,a.hash FROM catalog_packages p JOIN catalog_versions v ON v.package_slug=p.slug JOIN catalog_version_artifacts x ON x.version_id=v.id AND x.role='rules' JOIN catalog_artifacts a ON a.hash=x.artifact_hash WHERE p.slug=$1 ORDER BY v.created_at DESC,v.id DESC LIMIT 100").bind(&slug).fetch_all(&app.db).await?;
    if rows.is_empty() {
        return Err(Error::not_found());
    }
    Ok(Json(
        json!({"slug":slug,"publisher":rows[0].get::<String,_>("publisher"),"owned":rows[0].get::<Option<Uuid>,_>("owner_id")==Some(user),"versions":rows.iter().map(|r|json!({"id":r.get::<Uuid,_>("id"),"version":r.get::<String,_>("version"),"manifest":r.get::<Value,_>("manifest"),"rules":r.get::<Value,_>("content"),"hash":r.get::<String,_>("hash")})).collect::<Vec<_>>()}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publication {
    pub slug: String,
    pub version: String,
    pub manifest: Manifest,
    pub rules: Vec<Rule>,
}
pub async fn publish(
    State(app): State<App>,
    h: HeaderMap,
    Json(p): Json<Publication>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, true).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let publisher: Option<String> =
        sqlx::query_scalar("SELECT display_name FROM catalog_publishers WHERE user_id=$1")
            .bind(user)
            .fetch_optional(&mut *tx)
            .await?;
    let publisher = publisher
        .ok_or_else(|| Error::new(StatusCode::FORBIDDEN, "publisher approval required"))?;
    auth::rate(&app, format!("publish:{user}"), 20, 3600).await?;
    if p.slug.len() > 80
        || p.slug.is_empty()
        || !p
            .slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || p.version.len() > 40
        || p.version.split('.').count() != 3
        || !p
            .version
            .split('.')
            .all(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(bad("invalid slug or version (use x.y.z)"));
    }
    validate_manifest(&p.manifest)?;
    validate_rules(&p.rules).map_err(bad)?;
    sqlx::query("INSERT INTO catalog_packages(slug,owner_id,publisher) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(&p.slug).bind(user).bind(publisher).execute(&mut *tx).await?;
    let owner: Option<Uuid> =
        sqlx::query_scalar("SELECT owner_id FROM catalog_packages WHERE slug=$1 FOR UPDATE")
            .bind(&p.slug)
            .fetch_one(&mut *tx)
            .await?;
    if owner != Some(user) {
        return Err(Error::new(
            StatusCode::FORBIDDEN,
            "package owned by another publisher",
        ));
    }
    let rules = json!(p.rules);
    let manifest = json!(p.manifest);
    let hash = camofy::digest(serde_json::to_vec(&rules)?);
    if let Some(row)=sqlx::query("SELECT v.id,v.manifest,x.artifact_hash FROM catalog_versions v JOIN catalog_version_artifacts x ON x.version_id=v.id AND x.role='rules' WHERE v.package_slug=$1 AND v.version=$2").bind(&p.slug).bind(&p.version).fetch_optional(&mut *tx).await? {
        if row.get::<Value,_>("manifest")!=manifest || row.get::<String,_>("artifact_hash")!=hash {return Err(Error::new(StatusCode::CONFLICT,"published versions are immutable"));}
        return Ok(Json(json!({"id":row.get::<Uuid,_>("id"),"hash":hash})));
    }
    sqlx::query("INSERT INTO catalog_artifacts(hash,content) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(&hash)
        .bind(rules)
        .execute(&mut *tx)
        .await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO catalog_versions(id,package_slug,version,manifest) VALUES($1,$2,$3,$4)",
    )
    .bind(id)
    .bind(&p.slug)
    .bind(&p.version)
    .bind(manifest)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO catalog_version_artifacts(version_id,artifact_hash,role) VALUES($1,$2,'rules')").bind(id).bind(&hash).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"hash":hash})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installation {
    pub version_id: Uuid,
    pub profile_id: Uuid,
}
pub async fn install(
    State(app): State<App>,
    h: HeaderMap,
    Json(i): Json<Installation>,
) -> Result<Json<Resource>, Error> {
    let user = auth::user(&app, &h, true).await?;
    auth::rate(&app, format!("install:{user}"), 60, 60).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM resources WHERE id=$1 AND user_id=$2)")
            .bind(i.profile_id)
            .bind(user)
            .fetch_one(&mut *tx)
            .await?;
    if exists {
        let old = store::get(&app, &mut tx, user, i.profile_id).await?;
        if old.data["store"]["version_id"] == i.version_id.to_string() {
            return Ok(Json(old));
        }
        return Err(Error::new(
            StatusCode::CONFLICT,
            "installation ID already exists",
        ));
    }
    let row = sqlx::query("SELECT package_slug,manifest FROM catalog_versions WHERE id=$1")
        .bind(i.version_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(Error::not_found)?;
    let manifest: Value = row.get("manifest");
    let mut r = Resource {
        id: i.profile_id,
        kind: "profile".into(),
        version: 1,
        data: json!({"name":manifest["name"],"type":"overlay","origin":"store","store":{"version_id":i.version_id,"slug":row.get::<String,_>("package_slug"),"update_policy":"manual"}}),
    };
    // Check global UUID occupancy as well; never overwrite another tenant's resource.
    let used: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM resources WHERE id=$1)")
        .bind(i.profile_id)
        .fetch_one(&mut *tx)
        .await?;
    if used {
        return Err(Error::new(
            StatusCode::CONFLICT,
            "installation ID unavailable",
        ));
    }
    store::put(&app, &mut tx, user, &r).await?;
    hydrate(&mut tx, std::slice::from_mut(&mut r)).await?;
    store::notify(&mut tx, user).await?;
    tx.commit().await?;
    Ok(Json(r))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Upgrade {
    pub version: i64,
    pub version_id: Uuid,
    pub preview_digest: Option<String>,
}
pub async fn upgrade_preview(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(u): Json<Upgrade>,
) -> Result<Json<Value>, Error> {
    upgrade_inner(app, h, id, u, false).await
}
pub async fn upgrade(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(u): Json<Upgrade>,
) -> Result<Json<Value>, Error> {
    upgrade_inner(app, h, id, u, true).await
}
async fn upgrade_inner(
    app: App,
    h: HeaderMap,
    id: Uuid,
    u: Upgrade,
    commit: bool,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, true).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let mut records = store::list(&app, &mut tx, user).await?;
    let index = records
        .iter()
        .position(|r| r.id == id && r.kind == "profile" && r.data["store"].is_object())
        .ok_or_else(Error::not_found)?;
    if records[index].version != u.version {
        return Err(Error::new(StatusCode::CONFLICT, "profile changed; reload"));
    }
    let old_rules = records[index].data["_package"]["rules"].clone();
    let slug: Option<String> =
        sqlx::query_scalar("SELECT package_slug FROM catalog_versions WHERE id=$1")
            .bind(u.version_id)
            .fetch_optional(&mut *tx)
            .await?;
    if slug.as_deref() != records[index].data["store"]["slug"].as_str() {
        return Err(bad("version must belong to this package"));
    }
    records[index].data["store"]["version_id"] = json!(u.version_id);
    records[index].data["store"]["update_policy"] = json!("manual");
    records[index].version += 1;
    hydrate(&mut tx, std::slice::from_mut(&mut records[index])).await?;
    let mut affected = Vec::new();
    for b in records.iter().filter(|r| {
        r.kind == "bundle"
            && r.data["profiles"].as_array().is_some_and(|bs| {
                bs.iter()
                    .any(|b| b["enabled"] == true && b["profile_id"] == id.to_string())
            })
    }) {
        let (a, _) = store::render_bundle(&records, &b.data, &app.origin).map_err(bad)?;
        affected.push(json!({"id":b.id,"name":b.data["name"],"hash":a["router"]["hash"],"warnings":diagnostics(a["router"]["content"].as_str().unwrap()),"outputs":a.as_object().unwrap().iter().map(|(f,v)|(f.clone(),json!({"error":v["error"]}))).collect::<serde_json::Map<_,_>>()}));
    }
    let new_rules = &records[index].data["_package"]["rules"];
    let old: HashSet<String> = old_rules
        .as_array()
        .unwrap()
        .iter()
        .map(Value::to_string)
        .collect();
    let new: HashSet<String> = new_rules
        .as_array()
        .unwrap()
        .iter()
        .map(Value::to_string)
        .collect();
    let digest = camofy::digest(serde_json::to_vec(&json!([
        id,
        u.version,
        u.version_id,
        &affected,
        &records[index].data["_package"]
    ]))?);
    if commit {
        if u.preview_digest.as_deref() != Some(&digest) {
            return Err(Error::new(
                StatusCode::CONFLICT,
                "preview changed; preview again before applying",
            ));
        }
        store::put(&app, &mut tx, user, &records[index]).await?;
        let ids = affected
            .iter()
            .filter_map(|b| b["id"].as_str()?.parse().ok())
            .collect::<Vec<Uuid>>();
        store::rebuild_selected(&app, &mut tx, user, Some(&ids)).await?;
        // An unused installation still changed, even if no identity was rebuilt.
        if ids.is_empty() {
            store::notify(&mut tx, user).await?;
        }
        tx.commit().await?;
    }
    Ok(Json(
        json!({"preview_digest":digest,"affected":affected,"added":new.difference(&old).count(),"removed":old.difference(&new).count(),"applied":commit}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePreview {
    pub version: i64,
    pub policy: Option<String>,
}
/// Read-only compilation of an installed component, not a complete identity output.
pub async fn source_preview(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(p): Json<SourcePreview>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, true).await?;
    let mut conn = app.db.acquire().await?;
    let r = store::get(&app, &mut conn, user, id).await?;
    if !r.data["store"].is_object() {
        return Err(bad("managed profile required"));
    }
    if r.version != p.version {
        return Err(Error::new(StatusCode::CONFLICT, "profile changed; reload"));
    }
    let policy = p
        .policy
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(r.data["_package"]["manifest"]["default_policy"].as_str())
        .unwrap_or("<身份策略>");
    let content = compile(&r, &json!({"parameters":{"policy":policy}})).map_err(bad)?;
    let notice = notices(
        std::slice::from_ref(&r),
        &json!({"profiles":[{"profile_id":id,"enabled":true}]}),
    );
    Ok(Json(
        json!({"content":format!("{content}\n{notice}"),"policy":policy,"parameterized":policy=="<身份策略>","version":r.version}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fork {
    pub version: i64,
    pub policy: String,
}
pub async fn fork(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(f): Json<Fork>,
) -> Result<Json<Resource>, Error> {
    let user = auth::user(&app, &h, true).await?;
    auth::rate(&app, format!("install:{user}"), 60, 60).await?;
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let old = store::get(&app, &mut tx, user, id).await?;
    if !old.data["store"].is_object() || old.version != f.version {
        return Err(bad("managed profile version mismatch"));
    }
    let content = compile(&old, &json!({"parameters":{"policy":f.policy}})).map_err(bad)?;
    let r = Resource {
        id: Uuid::new_v4(),
        kind: "profile".into(),
        version: 1,
        data: json!({"name":format!("{} · 独立副本",old.data["name"].as_str().unwrap_or("Profile")),"type":"overlay","content":content,"provenance":old.data["_package"]["manifest"]["sources"]}),
    };
    store::put(&app, &mut tx, user, &r).await?;
    store::notify(&mut tx, user).await?;
    tx.commit().await?;
    Ok(Json(r))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub data: Value,
}
pub async fn identity_preview(
    State(app): State<App>,
    h: HeaderMap,
    Json(c): Json<Candidate>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, true).await?;
    let mut conn = app.db.acquire().await?;
    let records = store::list(&app, &mut conn, user).await?;
    let (a, _) = store::render_bundle(&records, &c.data, &app.origin).map_err(bad)?;
    let yaml = a["router"]["content"].as_str().unwrap();
    let v = camofy::engine::parse(yaml)?;
    let policies: Vec<String> = v["proxy-groups"]
        .as_sequence()
        .into_iter()
        .flatten()
        .chain(v["proxies"].as_sequence().into_iter().flatten())
        .filter_map(|g| g["name"].as_str().map(str::to_owned))
        .collect();
    Ok(Json(
        json!({"artifacts":a,"warnings":diagnostics(yaml),"policies":policies,"lock":lock_manifest(&records,&c.data)}),
    ))
}
