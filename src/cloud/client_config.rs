//! Request-specific views of immutable identity revisions. Devices continue to
//! download the exact artifact/hash advertised by their synchronization manifest.
use crate::{App, Error, auth, store};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use camofy::{
    compatibility, config_compat, config_compatibility,
    node_filter::{self, Policy, Report},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::Semaphore;
use uuid::Uuid;

pub const FORMATS: &[&str] = &[
    "auto",
    "agent",
    "clash",
    "router",
    "shadowrocket",
    "shadowrocket-nodes",
];

/// Format selection is separate from version-specific node capabilities. A known
/// product can select its format even when its exact core version is unverified.
pub fn resolve_format<'a>(
    requested: &'a str,
    client: &compatibility::Detection,
) -> anyhow::Result<&'a str> {
    anyhow::ensure!(FORMATS.contains(&requested), "unsupported output format");
    if requested != "auto" {
        if requested == "shadowrocket" && client.family.as_deref() == Some("shadowrocket") {
            anyhow::ensure!(
                !config_compatibility::full_configuration(client).blocked,
                "此版本尚不支持完整配置导入，请查看客户端兼容矩阵"
            );
        }
        return Ok(requested);
    }
    if client.family.as_deref() == Some("shadowrocket") {
        let assessment = config_compatibility::full_configuration(client);
        anyhow::ensure!(
            !assessment.blocked,
            "此版本尚不支持完整配置导入，请查看客户端兼容矩阵"
        );
        return Ok("shadowrocket");
    }
    match client.format.as_deref() {
        Some("clash") => {
            anyhow::ensure!(
                !config_compatibility::full_configuration(client).blocked,
                "此客户端的完整配置格式尚未确认，请查看客户端兼容矩阵"
            );
            Ok("clash")
        }
        None => Ok("router"),
        Some(_) => anyhow::bail!(
            "Auto 暂未提供此客户端的原生导出格式，请使用支持的客户端或明确选择输出格式"
        ),
    }
}

struct CachedView {
    key: String,
    artifact: Value,
    report: Report,
    bytes: usize,
}

static VIEW_CACHE: OnceLock<Mutex<VecDeque<CachedView>>> = OnceLock::new();

fn view_key(
    revision: Uuid,
    artifacts: &Value,
    format: &str,
    ua: Option<&str>,
) -> anyhow::Result<String> {
    // A published revision and compiler process use one immutable source lock.
    // Include the base artifact hash to protect callers using synthetic revisions.
    Ok(format!(
        "{revision}:{format}:{}:{}",
        artifacts["router"]["hash"],
        serde_json::to_string(&compatibility::detect(ua))?
    ))
}

fn cached_view(key: &str) -> Option<(Value, Report)> {
    let mut cache = VIEW_CACHE
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let index = cache.iter().position(|entry| entry.key == key)?;
    let entry = cache.remove(index).unwrap();
    let result = (entry.artifact.clone(), entry.report.clone());
    cache.push_back(entry);
    Some(result)
}

fn store_view(key: String, artifact: &Value, report: &Report) -> anyhow::Result<()> {
    let bytes =
        artifact["content"].as_str().map_or(0, str::len) + serde_json::to_vec(report)?.len();
    const MAX_BYTES: usize = 32 * 1024 * 1024;
    if bytes <= MAX_BYTES && !artifact["error"].is_string() {
        let mut cache = VIEW_CACHE
            .get_or_init(|| Mutex::new(VecDeque::new()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
    Ok(())
}
/// Small process-local LRU of immutable views. Never cache raw User-Agents or
/// authentication/usage decisions. Unrecognized UAs share one bounded key.
pub fn adapt_cached(
    revision: Uuid,
    artifacts: &Value,
    format: &str,
    ua: Option<&str>,
) -> anyhow::Result<(Value, Report)> {
    let key = view_key(revision, artifacts, format, ua)?;
    if let Some(result) = cached_view(&key) {
        return Ok(result);
    }
    let (artifact, report) = adapt(artifacts, format, ua)?;
    store_view(key, &artifact, &report)?;
    Ok((artifact, report))
}

/// Resolve rule dependencies outside identity/revision transactions. Published
/// requests and authenticated previews share the same compiler and source lock.
pub async fn adapt_request(
    app: &App,
    scope: crate::config_resources::Scope,
    artifacts: &Value,
    format: &str,
    ua: Option<&str>,
) -> anyhow::Result<(Value, Report)> {
    let client = compatibility::detect(ua);
    if !matches!(resolve_format(format, &client), Ok("shadowrocket")) {
        return match scope.revision {
            Some(revision) => adapt_cached(revision, artifacts, format, ua),
            None => adapt(artifacts, format, ua),
        };
    }
    // Authentication/ownership must precede even an in-process artifact hit.
    crate::config_resources::validate_scope(app, scope).await?;
    let key = scope
        .revision
        .map(|id| view_key(id, artifacts, format, ua))
        .transpose()?;
    if let Some(result) = key.as_deref().and_then(cached_view) {
        return Ok(result);
    }
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    let permit = tokio::time::timeout(
        Duration::from_secs(5),
        SLOTS
            .get_or_init(|| Arc::new(Semaphore::new(1)))
            .clone()
            .acquire_owned(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("完整配置转换繁忙，请稍后重试"))??;
    // Coalesce concurrent cache misses after the previous owner has finished.
    if let Some(result) = key.as_deref().and_then(cached_view) {
        return Ok(result);
    }
    let app = app.clone();
    let artifacts = artifacts.clone();
    let format = format.to_owned();
    let ua = ua.map(str::to_owned);
    let runtime = tokio::runtime::Handle::current();
    run_blocking_conversion(permit, move || {
        let base = artifacts["router"]["compatibility"]["base"]
            .as_str().or_else(|| artifacts["router"]["content"].as_str())
            .ok_or_else(|| anyhow::anyhow!("published identity has no YAML snapshot"))?;
        let config = camofy::engine::parse(base)?;
        // The blocking worker also performs source decoding/planning; the
        // runtime continues to drive bounded asynchronous fetches and leases.
        let resources = runtime.block_on(crate::config_resources::resolve(
            &app, scope, config_compat::Target::Shadowrocket, &config,
        ))?;
        let (mut artifact, report) = adapt_with_sources(&artifacts, &format, ua.as_deref(), &resources.sources)?;
        if artifact["configuration"].is_object() {
            artifact["configuration"]["resources"] = json!({
                "count": resources.source_count, "bytes": resources.total_bytes,
                "fingerprint": resources.fingerprint, "compiler_version": resources.compiler_version,
                "geosite_revision": resources.geosite_revision,
            });
        }
        if let Some(key) = key { store_view(key, &artifact, &report)?; }
        Ok((artifact, report))
    }).await
}

async fn run_blocking_conversion<T: Send + 'static>(
    permit: tokio::sync::OwnedSemaphorePermit,
    convert: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(move || {
        // Dropping the request future does not cancel a blocking task. Keep its
        // memory admission permit here until the actual computation has ended.
        let _permit = permit;
        convert()
    })
    .await
    .map_err(|_| anyhow::anyhow!("完整配置转换任务未完成，请稍后重试"))?
}

pub fn adapt(artifacts: &Value, format: &str, ua: Option<&str>) -> anyhow::Result<(Value, Report)> {
    adapt_with_sources(artifacts, format, ua, &Default::default())
}

fn adapt_with_sources(
    artifacts: &Value,
    format: &str,
    ua: Option<&str>,
    sources: &config_compat::ResolvedSources,
) -> anyhow::Result<(Value, Report)> {
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
    let resolved = resolve_format(format, &client);
    let filtered = node_filter::apply(
        &config,
        &Policy {
            auto: policy.auto,
            exclude_types: vec![],
        },
        client,
    )?;
    let mut report = filtered.report;
    if format == "auto" {
        if report.client.family.is_none() {
            report.warnings.push(
                "未识别客户端，Auto 返回完整 YAML；请在客户端中更新订阅，或指定输出格式。".into(),
            );
        } else if matches!(&resolved, Ok("shadowrocket")) {
            report
                .warnings
                .extend(config_compatibility::full_configuration(&report.client).warnings);
        }
    }
    if let Ok(manual) = serde_json::from_value::<Report>(snapshot["manual_report"].clone()) {
        report.before = manual.before;
        report.removed += manual.removed;
        report.repaired_references += manual.repaired_references;
        report.blocked_groups.splice(0..0, manual.blocked_groups);
        report.exclusions.splice(0..0, manual.exclusions);
        report.exclusions.truncate(200);
    }
    let format = match resolved {
        Ok(value) => value,
        Err(error) => return Ok((json!({"error":error.to_string(),"format":"auto"}), report)),
    };
    // Always compile request views with the current exporter. Published errors
    // and unsafe legacy graph shapes must not survive an exporter fix. Immutable
    // agent downloads use the separate revision endpoint and never pass here.
    let mut configuration = Value::Null;
    let content = match format {
        "router" | "agent" => camofy::engine::mihomo(&filtered.config, true),
        "clash" => {
            let capabilities = inspect_configuration(&filtered.config, &report.client);
            configuration = json!({"complete":true,"capabilities":capabilities});
            ensure_configuration_capabilities(&capabilities)
                .and_then(|()| camofy::engine::mihomo(&filtered.config, false))
        },
        "shadowrocket" => config_compat::compile(&filtered.config, config_compat::Target::Shadowrocket, sources)
            .and_then(|compiled| {
                let target_client = if report.client.family.as_deref() == Some("shadowrocket") {
                    report.client.clone()
                } else { compatibility::detect(Some("Shadowrocket")) };
                let capabilities = inspect_configuration(&compiled.config, &target_client);
                ensure_configuration_capabilities(&capabilities)?;
                configuration = json!({"stats":compiled.stats,"diagnostics":compiled.diagnostics,
                    "complete":true,"capabilities":capabilities,"source_hashes":compiled.source_hashes});
                camofy::engine::mihomo(&compiled.config, false)
            }),
        "shadowrocket-nodes" => camofy::engine::shadowrocket_nodes(&filtered.config),
        _ => unreachable!(),
    };
    let content = match content {
        Ok(value) => value,
        Err(error) => return Ok((json!({"error":error.to_string(),"format":format}), report)),
    };
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
        content.len() <= 16 * 1024 * 1024,
        "converted output exceeds 16 MiB"
    );
    Ok((
        json!({"hash":camofy::digest(content.as_bytes()),"content":content,"format":format,"configuration":configuration}),
        report,
    ))
}

/// Report only canonical feature identifiers, never configuration values or URLs.
fn inspect_configuration(
    config: &serde_yaml::Value,
    client: &compatibility::Detection,
) -> Vec<Value> {
    use std::collections::BTreeSet;
    let mut keys = BTreeSet::from(["syntax.clash_yaml".to_owned()]);
    keys.extend(config_compat::rule_capabilities(config));
    for provider in config["rule-providers"]
        .as_mapping()
        .into_iter()
        .flat_map(|map| map.values())
    {
        if let Some(kind) = provider["type"]
            .as_str()
            .filter(|kind| ["http", "file", "inline"].contains(kind))
        {
            keys.insert(format!("clash.provider.{kind}"));
        }
        if let Some(behavior) = provider["behavior"]
            .as_str()
            .filter(|kind| ["domain", "classical", "ipcidr"].contains(kind))
        {
            let format = provider["format"].as_str().unwrap_or("yaml");
            if ["yaml", "text", "mrs"].contains(&format) {
                keys.insert(format!("clash.provider.{behavior}_{format}"));
            }
        }
    }
    for group in config["proxy-groups"].as_sequence().into_iter().flatten() {
        keys.insert("clash.proxy_groups".into());
        if let Some(kind) = group["type"]
            .as_str()
            .filter(|s| ["select", "url-test", "fallback", "load-balance", "relay"].contains(s))
        {
            keys.insert(format!("clash.group.{}", kind.replace('-', "_")));
        }
    }
    for (field, key) in [
        ("nameserver", "nameserver"),
        ("nameserver-policy", "nameserver_policy"),
        ("fake-ip-filter", "fake_ip_filter"),
        ("fallback-filter", "fallback_filter"),
        ("proxy-server-nameserver", "proxy_server_nameserver"),
        ("respect-rules", "respect_rules"),
    ] {
        if !config["dns"][field].is_null() {
            keys.insert(format!("clash.dns.{key}"));
        }
    }
    if config["dns"]["nameserver-policy"]
        .as_mapping()
        .is_some_and(|map| {
            map.keys()
                .filter_map(serde_yaml::Value::as_str)
                .any(|key| key.starts_with("geosite:"))
        })
    {
        keys.insert("clash.dns.nameserver_policy_geosite".into());
    }
    if config["dns"]["fake-ip-filter"]
        .as_sequence()
        .is_some_and(|items| {
            items
                .iter()
                .filter_map(serde_yaml::Value::as_str)
                .any(|key| key.starts_with("geosite:"))
        })
    {
        keys.insert("clash.dns.fake_ip_filter_geosite".into());
    }
    keys.into_iter().map(|key| {
        let assessment = config_compatibility::capability(client, &key);
        json!({"key":key,"support":assessment.support,"basis":assessment.basis,"notes":assessment.notes,"evidence":assessment.evidence})
    }).collect()
}

fn ensure_configuration_capabilities(capabilities: &[Value]) -> anyhow::Result<()> {
    if let Some(item) = capabilities
        .iter()
        .find(|item| item["support"] == "unsupported")
    {
        anyhow::bail!(
            "当前客户端版本不支持完整配置中的 {}；未删除该配置",
            item["key"].as_str().unwrap_or("feature")
        );
    }
    Ok(())
}

pub async fn matrix(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>, Error> {
    auth::user(&app, &h, false).await?;
    let mut result = serde_json::to_value(compatibility::registry())?;
    result["configuration"] = serde_json::to_value(config_compatibility::registry())?;
    Ok(Json(result))
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
    let format = body.format.as_deref().unwrap_or("auto");
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
    let (artifact, report) = adapt_request(
        &app,
        crate::config_resources::Scope {
            user,
            bundle: id,
            revision: None,
        },
        &artifacts,
        format,
        body.user_agent.as_deref(),
    )
    .await
    .map_err(|e| Error::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    Ok(Json(
        json!({"content":artifact["content"],"error":artifact["error"],"format":artifact["format"],"configuration":artifact["configuration"],"report":report,"policy":b.data["node_filter"]}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_request_holds_capacity_until_blocking_work_finishes() {
        let slots = Arc::new(Semaphore::new(1));
        let permit = slots.clone().acquire_owned().await.unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let task = tokio::spawn(run_blocking_conversion(permit, move || {
            let _ = started.send(());
            wait.recv()
                .map_err(|_| anyhow::anyhow!("synthetic synchronization failed"))?;
            Ok(())
        }));
        ready.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), slots.clone().acquire_owned())
                .await
                .is_err()
        );
        release.send(()).unwrap();
        let _available = tokio::time::timeout(Duration::from_secs(2), slots.acquire_owned())
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn complete_cached_views_preserve_resource_metadata_and_reject_errors() {
        let artifacts = fixture("rules: ['MATCH,DIRECT']", true);
        let revision = Uuid::new_v4();
        let key = view_key(revision, &artifacts, "auto", Some("Shadowrocket/2.2.90")).unwrap();
        let (mut artifact, report) =
            adapt(&artifacts, "auto", Some("Shadowrocket/2.2.90")).unwrap();
        artifact["configuration"]["resources"] =
            json!({"count":1,"fingerprint":"synthetic-resource-lock"});
        store_view(key.clone(), &artifact, &report).unwrap();
        assert_eq!(cached_view(&key).unwrap().0, artifact);
        let error_key = view_key(
            Uuid::new_v4(),
            &artifacts,
            "auto",
            Some("Shadowrocket/2.2.90"),
        )
        .unwrap();
        store_view(
            error_key.clone(),
            &json!({"error":"synthetic conversion error"}),
            &report,
        )
        .unwrap();
        assert!(cached_view(&error_key).is_none());
        assert_ne!(
            key,
            view_key(
                revision,
                &artifacts,
                "shadowrocket",
                Some("Shadowrocket/2.2.90")
            )
            .unwrap()
        );
    }
    fn fixture(content: &str, auto: bool) -> Value {
        let config = camofy::engine::parse(content).unwrap();
        json!({"agent":{"hash":"immutable-device-hash","content":"immutable-device-content"},
            "router":{"hash":"old-compiler","content":content,"compatibility":{"base":serde_yaml::to_string(&config).unwrap(),"policy":{"auto":auto,"exclude_types":[]}}},
            "shadowrocket":{"error":"obsolete compiler error"},
            "shadowrocket-nodes":{"error":"obsolete compiler error"}})
    }

    #[test]
    fn auto_selects_format_separately_from_version_confidence() {
        for (ua, expected) in [
            (None, "router"),
            (Some("unknown/1.0"), "router"),
            (Some("mihomo/1.19.17"), "clash"),
            (Some("ClashMetaForAndroid/2.10.2.Meta"), "clash"),
            (Some("Stash/3.3.3"), "clash"),
            (Some("Shadowrocket/2.2.90"), "shadowrocket"),
            (Some("Shadowrocket/99.0.0"), "shadowrocket"),
        ] {
            let detection = compatibility::detect(ua);
            assert_eq!(resolve_format("auto", &detection).unwrap(), expected);
            assert_eq!(resolve_format("router", &detection).unwrap(), "router");
        }
        assert!(resolve_format("auto", &compatibility::detect(Some("sing-box/1.12.0"))).is_err());
    }

    #[test]
    fn auto_preserves_complete_configuration_without_uri_limits() {
        let content = "proxies:\n  - {name: Plain, type: ss, server: proxy.example, port: 443, cipher: aes-128-gcm, password: example}\n  - {name: Web, type: vless, server: proxy.example, port: 443, uuid: 00000000-0000-0000-0000-000000000001, network: ws, ws-opts: {headers: {Authorization: example}}}\nproxy-groups: [{name: Pick, type: select, proxies: [Plain, Web]}]\nrules: ['DOMAIN,service.example,Pick', 'MATCH,REJECT']\n";
        let artifacts = fixture(content, true);
        let before = artifacts.clone();
        let (view, report) = adapt(&artifacts, "auto", Some("Shadowrocket/99.0.0")).unwrap();
        assert_eq!(view["format"], "shadowrocket");
        assert_eq!(report.removed, 0);
        assert!(view["error"].is_null(), "{view}");
        let yaml = camofy::engine::parse(view["content"].as_str().unwrap()).unwrap();
        assert_eq!(yaml["proxies"].as_sequence().unwrap().len(), 2);
        assert_eq!(yaml["rules"].as_sequence().unwrap().len(), 2);
        assert_eq!(yaml["proxy-groups"].as_sequence().unwrap().len(), 1);
        assert_eq!(
            yaml["proxies"][1]["ws-opts"]["headers"]["Authorization"],
            "example"
        );
        assert_eq!(artifacts, before);
        assert!(adapt(&artifacts, "shadowrocket-nodes", None).unwrap().0["error"].is_string());
        assert!(
            adapt(
                &fixture(content, false),
                "auto",
                Some("Shadowrocket/99.0.0")
            )
            .unwrap()
            .0["error"]
                .is_null()
        );
    }

    #[test]
    fn auto_preserves_protocol_parameters_and_dialer_chains() {
        let content = "proxies:\n  - {name: Reality, type: vless, server: proxy.example, port: 443, uuid: 00000000-0000-0000-0000-000000000001, tls: true, udp: true, network: tcp, flow: xtls-rprx-vision, client-fingerprint: chrome, reality-opts: {public-key: AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA, short-id: ''}, alterId: 0, cipher: auto}\n  - {name: QUIC, type: hysteria2, server: proxy.example, port: 443, password: sample, up: null, down: null, hop-interval: 30, sni: front.example}\n  - {name: Auth, type: socks5, server: proxy.example, port: 1080, username: sample, password: sample, cipher: null, alterId: null}\n  - {name: Chained, type: socks5, server: proxy.example, port: 1080, username: sample, password: sample, dialer-proxy: Reality}\nproxy-groups: [{name: Choose, type: select, proxies: [Reality, QUIC, Auth, Chained]}]\nrules: ['MATCH,Choose']\n";
        let artifacts = fixture(content, true);
        let (view, report) = adapt(&artifacts, "auto", Some("Shadowrocket/2.2.90")).unwrap();
        assert!(view["error"].is_null(), "{view}");
        assert_eq!(report.removed, 0);
        assert_eq!(report.retained, 4);
        let yaml = camofy::engine::parse(view["content"].as_str().unwrap()).unwrap();
        assert_eq!(yaml["proxies"][3]["dialer-proxy"], "Reality");
        assert_eq!(yaml["proxies"][0]["flow"], "xtls-rprx-vision");
        assert!(yaml["proxies"][0]["reality-opts"].is_mapping());
        assert_eq!(yaml["rules"][0], "MATCH,Choose");
    }

    #[test]
    fn configuration_matrix_checks_actual_fields_without_confusing_policy_names() {
        let artifacts = fixture(
            "proxy-groups: [{name: AND, type: select, proxies: [REJECT]}]\nrules: ['DOMAIN,service.example,AND', 'MATCH,AND']",
            true,
        );
        let (artifact, _) = adapt(&artifacts, "auto", Some("Clash/1.18.0")).unwrap();
        assert!(artifact["error"].is_null(), "{artifact}");
        assert!(
            !artifact["configuration"]["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["key"] == "clash.rule.and")
        );
        let artifacts = fixture(
            "proxy-groups: [{name: Pick, type: relay, proxies: [DIRECT]}]\nrules: ['MATCH,Pick']",
            true,
        );
        let (artifact, _) = adapt(&artifacts, "auto", Some("mihomo/1.19.17")).unwrap();
        assert!(
            artifact["error"]
                .as_str()
                .unwrap()
                .contains("clash.group.relay")
        );
        let (raw, _) = adapt(&artifacts, "router", Some("mihomo/1.19.17")).unwrap();
        assert!(raw["error"].is_null());
    }

    #[test]
    fn auto_expands_inline_rules_without_changing_policy_or_dns() {
        let artifacts = fixture(
            "proxies: []\nproxy-groups: [{name: Pick, type: select, proxies: [REJECT]}]\nrule-providers: {example: {type: inline, behavior: classical, payload: ['DOMAIN-SUFFIX,example.test', 'IP-CIDR,192.0.2.0/24']}}\ndns: {enable: true, nameserver: ['https://dns.example/dns-query']}\nrules: ['DOMAIN,first.test,DIRECT', 'RULE-SET,example,Pick,no-resolve', 'MATCH,REJECT']",
            true,
        );
        let (artifact, _) =
            adapt(&artifacts, "auto", Some("Shadowrocket/3131 CFNetwork/1")).unwrap();
        assert!(artifact["error"].is_null(), "{artifact}");
        let config = camofy::engine::parse(artifact["content"].as_str().unwrap()).unwrap();
        assert!(config.get("rule-providers").is_none());
        assert_eq!(config["rules"][0], "DOMAIN,first.test,DIRECT");
        assert_eq!(config["rules"][2], "IP-CIDR,192.0.2.0/24,Pick,no-resolve");
        assert_eq!(config["rules"][3], "MATCH,REJECT");
        assert_eq!(
            config["dns"]["nameserver"][0],
            "https://dns.example/dns-query"
        );
        assert_eq!(artifact["configuration"]["stats"]["output_rules"], 4);
    }

    #[test]
    fn corrected_exporters_replace_published_errors_without_republishing() {
        let artifacts = fixture(
            "proxies: [{name: Authenticated, type: socks5, server: proxy.example, port: 1080, username: sample, password: sample}]\nproxy-providers: {}\nrules: ['MATCH,Authenticated']\n",
            true,
        );
        let (view, report) = adapt(&artifacts, "shadowrocket", None).unwrap();
        assert_eq!(report.removed, 0);
        assert!(view["error"].is_null(), "{view}");
        assert!(
            view["content"]
                .as_str()
                .unwrap()
                .contains("username: sample")
        );
        assert_eq!(artifacts["agent"]["hash"], "immutable-device-hash");
    }

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
