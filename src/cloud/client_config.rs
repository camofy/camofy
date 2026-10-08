//! Request-specific views of immutable identity revisions. Devices continue to
//! download the exact artifact/hash advertised by their synchronization manifest.
use crate::{App, Error, auth, store};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use camofy::{
    compatibility,
    node_filter::{self, Policy, Report},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
};
use uuid::Uuid;

pub const FORMATS: &[&str] = &[
    "agent",
    "clash",
    "router",
    "shadowrocket",
    "shadowrocket-nodes",
];

struct CachedView {
    key: String,
    artifact: Value,
    report: Report,
    bytes: usize,
}
/// Small process-local LRU of immutable views. Never cache raw User-Agents or
/// authentication/usage decisions. Unrecognized UAs share one bounded key.
pub fn adapt_cached(
    revision: Uuid,
    artifacts: &Value,
    format: &str,
    ua: Option<&str>,
) -> anyhow::Result<(Value, Report)> {
    static CACHE: OnceLock<Mutex<VecDeque<CachedView>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    let key = format!(
        "{revision}:{format}:{}:{}",
        artifacts["router"]["hash"],
        serde_json::to_string(&compatibility::detect(ua))?
    );
    {
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(index) = cache.iter().position(|entry| entry.key == key) {
            let entry = cache.remove(index).unwrap();
            let result = (entry.artifact.clone(), entry.report.clone());
            cache.push_back(entry);
            return Ok(result);
        }
    }
    let (artifact, report) = adapt(artifacts, format, ua)?;
    let bytes =
        artifact["content"].as_str().map_or(0, str::len) + serde_json::to_vec(&report)?.len();
    const MAX_BYTES: usize = 8 * 1024 * 1024;
    if bytes <= MAX_BYTES && !artifact["error"].is_string() {
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        // Concurrent misses can calculate the same view; keep a single entry.
        cache.retain(|entry| entry.key != key);
        while cache.len() >= 12
            || cache.iter().map(|entry| entry.bytes).sum::<usize>() + bytes > MAX_BYTES
        {
            if cache.pop_front().is_none() {
                break;
            }
        }
        cache.push_back(CachedView {
            key,
            artifact: artifact.clone(),
            report: report.clone(),
            bytes,
        });
    }
    Ok((artifact, report))
}

pub fn adapt(artifacts: &Value, format: &str, ua: Option<&str>) -> anyhow::Result<(Value, Report)> {
    anyhow::ensure!(FORMATS.contains(&format), "unsupported output format");
    let snapshot = &artifacts["router"]["compatibility"];
    let policy = Policy::parse(&snapshot["policy"])?;
    let base = snapshot["base"]
        .as_str()
        .or_else(|| artifacts["router"]["content"].as_str())
        .ok_or_else(|| anyhow::anyhow!("published identity has no YAML snapshot"))?;
    let config = camofy::engine::parse(base)?;
    let client = compatibility::detect(ua);
    // Manual exclusions have already been applied to this revision's base.
    // Legacy revisions never contained them; both paths share the same engine.
    let filtered = node_filter::apply(
        &config,
        &Policy {
            auto: policy.auto,
            exclude_types: vec![],
        },
        client,
    )?;
    let changed = filtered.report.removed > 0;
    let mut report = filtered.report;
    if let Ok(manual) = serde_json::from_value::<Report>(snapshot["manual_report"].clone()) {
        report.before = manual.before;
        report.removed += manual.removed;
        report.repaired_references += manual.repaired_references;
        report.blocked_groups.splice(0..0, manual.blocked_groups);
        report.exclusions.splice(0..0, manual.exclusions);
        report.exclusions.truncate(200);
    }
    let original = artifacts
        .get(format)
        .ok_or_else(|| anyhow::anyhow!("output not found"))?;
    if !changed {
        return Ok((
            json!({"hash":original["hash"],"content":original["content"],"error":original["error"]}),
            report,
        ));
    }
    let content = match format {
        "router" | "agent" => camofy::engine::mihomo(&filtered.config, true),
        "clash" => camofy::engine::mihomo(&filtered.config, false),
        "shadowrocket" => camofy::engine::shadowrocket_full(&filtered.config),
        "shadowrocket-nodes" => camofy::engine::shadowrocket_nodes(&filtered.config),
        _ => unreachable!(),
    }?;
    let content = if format == "shadowrocket-nodes" {
        content
    } else {
        let notices = snapshot["notices"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                base.lines()
                    .take_while(|s| s.starts_with('#') || s.trim().is_empty())
                    .map(|s| format!("{s}\n"))
                    .collect()
            });
        format!("{notices}{content}")
    };
    anyhow::ensure!(
        content.len() <= 4 * 1024 * 1024,
        "merged output exceeds 4 MiB"
    );
    Ok((
        json!({"hash":camofy::digest(content.as_bytes()),"content":content}),
        report,
    ))
}

pub async fn matrix(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>, Error> {
    auth::user(&app, &h, false).await?;
    Ok(Json(serde_json::to_value(compatibility::registry())?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    pub user_agent: Option<String>,
    pub node_filter: Option<Value>,
    pub format: Option<String>,
}
pub async fn preview(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<Preview>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, true).await?;
    auth::rate(&app, format!("compatibility-preview:{user}"), 30, 60).await?;
    if body
        .user_agent
        .as_ref()
        .is_some_and(|u| u.len() > 512 || u.chars().any(char::is_control))
    {
        return Err(Error::bad("User-Agent 最多 512 字节，不能包含控制字符"));
    }
    let format = body.format.as_deref().unwrap_or("router");
    if !FORMATS.contains(&format) {
        return Err(Error::bad("unsupported output format"));
    }
    let mut conn = app.db.acquire().await?;
    let mut b = store::get(&app, &mut conn, user, id).await?;
    if b.kind != "bundle" {
        return Err(Error::not_found());
    }
    let records = store::list(&app, &mut conn, user).await?;
    drop(conn);
    if let Some(policy) = body.node_filter {
        b.data["node_filter"] =
            serde_json::to_value(Policy::parse(&policy).map_err(|e| Error::bad(e.to_string()))?)?;
    }
    let (artifacts, _) = store::render_bundle(&records, &b.data, &app.origin)
        .map_err(|e| Error::bad(e.to_string()))?;
    let (artifact, report) = adapt(&artifacts, format, body.user_agent.as_deref())
        .map_err(|e| Error::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    Ok(Json(
        json!({"content":artifact["content"],"error":artifact["error"],"report":report,"policy":b.data["node_filter"]}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immutable_artifacts_are_varied_per_client_and_policy_is_pinned() {
        let p = store::Resource {
            id: Uuid::new_v4(),
            kind: "profile".into(),
            version: 1,
            data: json!({"type":"overlay","content":"proxies: [{name: old, type: ss, server: proxy.example, port: 443}, {name: new, type: mieru, server: proxy.example, port: 443}]\nproxy-groups: [{name: Pick, type: select, proxies: [new, old]}]\nrules: ['MATCH,Pick']"}),
        };
        let data = json!({"profiles":[{"profile_id":p.id,"enabled":true}]});
        let (artifacts, _) =
            store::render_bundle(std::slice::from_ref(&p), &data, "https://cloud.example").unwrap();
        let (old, report) = adapt(
            &artifacts,
            "router",
            Some("ClashMetaForAndroid/2.10.2.Meta"),
        )
        .unwrap();
        assert_eq!(report.removed, 1);
        assert!(!old["content"].as_str().unwrap().contains("type: mieru"));
        let (new, report) = adapt(&artifacts, "router", Some("mihomo/1.19.17")).unwrap();
        assert_eq!(report.removed, 0);
        assert_eq!(new["hash"], artifacts["router"]["hash"]);
        assert_ne!(old["hash"], new["hash"]);
        let mut disabled = data.clone();
        disabled["node_filter"] = json!({"auto":false,"exclude_types":[]});
        let (disabled, _) = store::render_bundle(&[p], &disabled, "https://cloud.example").unwrap();
        assert_eq!(
            adapt(&disabled, "router", Some("ClashMetaForAndroid/2.10.2.Meta"))
                .unwrap()
                .1
                .removed,
            0
        );
        let mut legacy = artifacts.clone();
        legacy["router"]
            .as_object_mut()
            .unwrap()
            .remove("compatibility");
        assert_eq!(
            adapt(&legacy, "router", Some("ClashMetaForAndroid/2.10.2.Meta"))
                .unwrap()
                .1
                .removed,
            1
        );
        assert!(adapt(&artifacts, "unknown", None).is_err());
        let revision = Uuid::new_v4();
        let cached = adapt_cached(
            revision,
            &artifacts,
            "router",
            Some("ClashMetaForAndroid/2.10.2.Meta"),
        )
        .unwrap();
        assert_eq!(cached.0, old);
        assert_eq!(
            adapt_cached(revision, &artifacts, "router", Some("mihomo/1.19.17"))
                .unwrap()
                .0["hash"],
            new["hash"]
        );
        assert_eq!(
            adapt_cached(
                Uuid::new_v4(),
                &disabled,
                "router",
                Some("ClashMetaForAndroid/2.10.2.Meta")
            )
            .unwrap()
            .1
            .removed,
            0
        );
    }
}
