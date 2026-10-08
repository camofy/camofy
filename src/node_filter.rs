//! One graph transformation shared by explicit exclusions and client adaptation.
//! Removed upstreams must never turn a chain into a direct connection.
use crate::compatibility::{self, Detection, Support};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub auto: bool,
    pub exclude_types: Vec<String>,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            auto: true,
            exclude_types: vec![],
        }
    }
}
impl Policy {
    pub fn parse(value: &serde_json::Value) -> Result<Self> {
        let mut policy: Self = if value.is_null() {
            Self::default()
        } else {
            serde_json::from_value(value.clone())?
        };
        ensure!(policy.exclude_types.len() <= 64, "最多排除 64 种协议");
        for kind in &mut policy.exclude_types {
            *kind = canonical_type(kind);
            ensure!(
                !kind.is_empty()
                    && kind.len() <= 40
                    && kind
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "节点类型必须是协议标识"
            );
        }
        policy.exclude_types.sort();
        policy.exclude_types.dedup();
        Ok(policy)
    }
}
pub fn canonical_type(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "shadowsocks" => "ss".into(),
        "shadowsocksr" => "ssr".into(),
        "hy2" => "hysteria2".into(),
        "socks" => "socks5".into(),
        v => v.into(),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Exclusion {
    pub name: String,
    pub protocol: String,
    pub reason: String,
    pub capability: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Report {
    pub matrix_version: String,
    pub client: Detection,
    pub before: usize,
    pub retained: usize,
    pub removed: usize,
    pub exclusions: Vec<Exclusion>,
    pub repaired_references: usize,
    pub blocked_groups: Vec<String>,
    pub warnings: Vec<String>,
    pub unknown_capabilities: BTreeSet<String>,
}
pub struct Filtered {
    pub config: Value,
    pub report: Report,
}
fn text<'a>(v: &'a Value, field: &str) -> &'a str {
    v.get(field).and_then(Value::as_str).unwrap_or("")
}
fn present(v: &Value, key: &str) -> bool {
    v.get(key).is_some_and(|v| !v.is_null())
}
fn requirements(node: &Value, kind: &str) -> Vec<String> {
    let mut requirements = vec![];
    if ["vless", "vmess", "trojan"].contains(&kind) {
        let network = text(node, "network");
        if !network.is_empty() && network != "tcp" {
            requirements.push(format!("{kind}.{network}"));
        }
        if present(node, "reality-opts") {
            requirements.push(format!("{kind}.reality"));
        }
        if !text(node, "flow").is_empty() {
            requirements.push(format!("{kind}.{}", text(node, "flow")));
        }
    }
    if kind == "mieru" {
        if text(node, "transport").eq_ignore_ascii_case("udp") {
            requirements.push("mieru.udp-transport".into());
        }
        if node.get("udp").and_then(Value::as_bool) == Some(true) {
            requirements.push("mieru.udp-relay".into());
        }
        for field in ["multiplexing", "handshake-mode", "traffic-pattern"] {
            if present(node, field) {
                requirements.push(format!("mieru.{field}"));
            }
        }
    }
    if kind == "ss" && text(node, "cipher").starts_with("2022-") {
        requirements.push("ss.2022".into());
    }
    if present(node, "plugin") {
        requirements.push(format!("{kind}.plugin.{}", text(node, "plugin")));
    }
    if kind == "snell"
        && let Some(v) = node.get("version").and_then(Value::as_u64)
    {
        requirements.push(format!("snell.v{v}"));
    }
    if present(node, "ech-opts") {
        requirements.push("tls.ech".into());
    }
    requirements
}
fn reason(
    node: &Value,
    policy: &Policy,
    detection: &Detection,
    report: &mut Report,
) -> Option<(String, String)> {
    let kind = canonical_type(text(node, "type"));
    if policy.exclude_types.contains(&kind) {
        return Some(("manual".into(), kind));
    }
    if !policy.auto {
        return None;
    }
    let profile = detection.capabilities()?;
    match profile.protocol(&kind) {
        Support::Unsupported => return Some(("unsupported_protocol".into(), kind)),
        Support::Unknown => {
            report
                .unknown_capabilities
                .insert(format!("protocol.{kind}"));
        }
        Support::Supported => {}
    }
    for feature in requirements(node, &kind) {
        match profile.feature(&feature) {
            Support::Unsupported => return Some(("unsupported_feature".into(), feature)),
            Support::Unknown => {
                report.unknown_capabilities.insert(feature);
            }
            Support::Supported => {}
        }
    }
    None
}
fn record(node: &Value, cause: (String, String), report: &mut Report) {
    report.removed += 1;
    if report.exclusions.len() < 200 {
        report.exclusions.push(Exclusion {
            name: text(node, "name").into(),
            protocol: canonical_type(text(node, "type")),
            reason: cause.0,
            capability: cause.1,
        });
    }
}
fn prune(
    list: &mut Vec<Value>,
    policy: &Policy,
    client: &Detection,
    report: &mut Report,
    removed: &mut BTreeSet<String>,
) {
    list.retain(|node| {
        report.before += 1;
        if let Some(cause) = reason(node, policy, client, report) {
            removed.insert(text(node, "name").into());
            record(node, cause, report);
            false
        } else {
            true
        }
    });
}
fn block_group(group: &mut Value, report: &mut Report) {
    let name = text(group, "name").to_owned();
    *group =
        serde_yaml::to_value(serde_json::json!({"name":name,"type":"select","proxies":["REJECT"]}))
            .unwrap();
    report.blocked_groups.push(name);
}
fn repair_rules(rules: &mut Value, removed: &BTreeSet<String>, report: &mut Report) {
    if let Some(list) = rules.as_sequence_mut() {
        for rule in list {
            let Some(s) = rule.as_str() else {
                continue;
            };
            // The policy is the final top-level token (before no-resolve).
            // Nested logical conditions remain byte-for-byte intact.
            if s.starts_with("SUB-RULE,") {
                continue;
            }
            let suffix = s.trim_end().ends_with(",no-resolve");
            let end = if suffix {
                s.rfind(',').unwrap()
            } else {
                s.len()
            };
            let start = s[..end].rfind(',').map_or(0, |at| at + 1);
            if removed.contains(s[start..end].trim()) {
                *rule = Value::String(format!("{}REJECT{}", &s[..start], &s[end..]));
                report.repaired_references += 1;
            }
        }
    }
}
pub fn apply(input: &Value, policy: &Policy, client: Detection) -> Result<Filtered> {
    let mut config = input.clone();
    let mut report = Report {
        matrix_version: compatibility::registry().version.clone(),
        client: client.clone(),
        ..Default::default()
    };
    if policy.auto && client.capabilities().is_none() {
        report
            .warnings
            .push("客户端或版本能力未确认，自动模式保留未知节点；指定类型排除仍然生效。".into());
    }
    if client.format.as_deref().is_some_and(|f| f != "clash") {
        report.warnings.push(
            "此客户端使用独立配置格式。节点协议过滤不会把 Clash YAML 转换为其原生完整配置。".into(),
        );
    }
    let mut removed = BTreeSet::new();
    if let Some(nodes) = config.get_mut("proxies").and_then(Value::as_sequence_mut) {
        prune(nodes, policy, &client, &mut report, &mut removed);
    }
    let mut empty_providers = BTreeSet::new();
    let mut changed_providers = BTreeSet::new();
    let mut remote_providers = 0;
    if let Some(providers) = config
        .get_mut("proxy-providers")
        .and_then(Value::as_mapping_mut)
    {
        for (name, provider) in providers {
            if text(provider, "type") == "inline" {
                if let Some(nodes) = provider.get_mut("payload").and_then(Value::as_sequence_mut) {
                    let before = nodes.len();
                    prune(nodes, policy, &client, &mut report, &mut BTreeSet::new());
                    if nodes.len() != before {
                        changed_providers.insert(name.as_str().unwrap_or("").to_owned());
                    }
                    if nodes.is_empty() {
                        empty_providers.insert(name.as_str().unwrap_or("").to_owned());
                    }
                }
            } else {
                remote_providers += 1;
            }
        }
    }
    if remote_providers > 0 && !policy.exclude_types.is_empty() {
        // External data cannot be inspected by this pure transformation. A
        // passing result must not claim that later provider downloads are safe.
        anyhow::bail!(
            "配置包含远程或文件代理集合，无法按类型排除其中节点。请先将其作为订阅源导入并合并为内联节点。"
        );
    }
    if remote_providers > 0 && policy.auto {
        report.warnings.push("远程或文件代理集合在客户端下载时才展开，其中节点尚未检查；可将它们作为订阅源导入以参与自动兼容。".into());
    }
    // Removing a dialer requires removing all nodes whose connectivity depends
    // on it. A bounded fixed point follows the existing 64-level graph limit.
    for _ in 0..=64 {
        let previous = removed.len();
        if let Some(nodes) = config.get_mut("proxies").and_then(Value::as_sequence_mut) {
            nodes.retain(|node| {
                if removed.contains(text(node, "dialer-proxy")) {
                    removed.insert(text(node, "name").into());
                    record(
                        node,
                        ("dependency".into(), "dialer-proxy".into()),
                        &mut report,
                    );
                    false
                } else {
                    true
                }
            });
        }
        if previous == removed.len() {
            break;
        }
    }
    // Inline provider nodes have their own names but may dial through a global
    // outbound. Do not leave those references dangling after pruning globals.
    if let Some(providers) = config
        .get_mut("proxy-providers")
        .and_then(Value::as_mapping_mut)
    {
        for (name, provider) in providers {
            if text(provider, "type") == "inline"
                && let Some(nodes) = provider.get_mut("payload").and_then(Value::as_sequence_mut)
            {
                let before = nodes.len();
                nodes.retain(|node| {
                    if removed.contains(text(node, "dialer-proxy")) {
                        record(
                            node,
                            ("dependency".into(), "dialer-proxy".into()),
                            &mut report,
                        );
                        false
                    } else {
                        true
                    }
                });
                if nodes.len() != before {
                    changed_providers.insert(name.as_str().unwrap_or("").to_owned());
                }
                if nodes.is_empty() {
                    empty_providers.insert(name.as_str().unwrap_or("").to_owned());
                }
            }
        }
    }
    if let Some(groups) = config
        .get_mut("proxy-groups")
        .and_then(Value::as_sequence_mut)
    {
        for group in groups {
            let relay = text(group, "type") == "relay";
            let mut changed = false;
            if let Some(members) = group.get_mut("proxies").and_then(Value::as_sequence_mut) {
                let before = members.len();
                members.retain(|v| !v.as_str().is_some_and(|n| removed.contains(n)));
                changed = before != members.len();
                report.repaired_references += before - members.len();
            }
            if let Some(providers) = group.get_mut("use").and_then(Value::as_sequence_mut) {
                changed |= relay
                    && providers
                        .iter()
                        .any(|v| v.as_str().is_some_and(|n| changed_providers.contains(n)));
                let before = providers.len();
                providers.retain(|v| !v.as_str().is_some_and(|n| empty_providers.contains(n)));
                changed |= before != providers.len();
                report.repaired_references += before - providers.len();
            }
            let has_members = group
                .get("proxies")
                .and_then(Value::as_sequence)
                .is_some_and(|s| !s.is_empty());
            let has_providers = group
                .get("use")
                .and_then(Value::as_sequence)
                .is_some_and(|s| !s.is_empty())
                || group.get("include-all").and_then(Value::as_bool) == Some(true)
                || group.get("include-all-proxies").and_then(Value::as_bool) == Some(true)
                || group.get("include-all-providers").and_then(Value::as_bool) == Some(true);
            if relay && has_providers && (!removed.is_empty() || !changed_providers.is_empty()) {
                changed = true;
            }
            if changed && (relay || !has_members && !has_providers) {
                block_group(group, &mut report);
            }
        }
    }
    if let Some(rules) = config.get_mut("rules") {
        repair_rules(rules, &removed, &mut report);
    }
    if let Some(subrules) = config.get_mut("sub-rules").and_then(Value::as_mapping_mut) {
        for rules in subrules.values_mut() {
            repair_rules(rules, &removed, &mut report);
        }
    }
    // Provider fetch paths and DNS routing must not silently lose their proxy.
    for field in ["proxy-providers", "rule-providers"] {
        if let Some(providers) = config.get_mut(field).and_then(Value::as_mapping_mut) {
            for provider in providers.values_mut() {
                if removed.contains(text(provider, "proxy")) {
                    provider["proxy"] = Value::String("REJECT".into());
                    report.repaired_references += 1;
                }
            }
        }
    }
    if !removed.is_empty()
        && let Some(dns) = config.get("dns")
    {
        // DNS URL fragments may select an outbound; arbitrary DNS URL grammar
        // must not be rewritten with a node-name string replacement.
        fn references(v: &Value, names: &BTreeSet<String>) -> bool {
            match v {
                Value::String(s) => s.split_once('#').is_some_and(|(_, f)| {
                    f.split('&').any(|x| {
                        names.contains(
                            percent_encoding::percent_decode_str(x)
                                .decode_utf8_lossy()
                                .as_ref(),
                        )
                    })
                }),
                Value::Sequence(v) => v.iter().any(|x| references(x, names)),
                Value::Mapping(v) => v.values().any(|x| references(x, names)),
                _ => false,
            }
        }
        ensure!(
            !references(dns, &removed),
            "DNS 配置引用了被排除的节点，请改用保留的代理组后再过滤"
        );
    }
    report.retained = report.before - report.removed;
    ensure!(
        report.before == 0 || report.retained > 0 || remote_providers > 0,
        "过滤后没有可用节点，已阻止下发空订阅；请调整该身份的过滤设置或订阅源"
    );
    crate::engine::validate(&config)?;
    Ok(Filtered { config, report })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Value {
        crate::engine::parse("proxies:\n- {name: modern, type: mieru, server: proxy.example, port: 443}\n- {name: future, type: anytls, server: proxy.example, port: 443}\n- {name: legacy, type: ss, server: proxy.example, port: 443}\n- {name: chained, type: socks5, server: proxy.example, port: 1080, dialer-proxy: modern}\nproxy-groups:\n- {name: Choice, type: select, proxies: [modern, future, legacy, chained]}\n- {name: Empty, type: url-test, proxies: [modern]}\n- {name: Chain, type: relay, proxies: [modern, legacy]}\nrules: ['DOMAIN,example.com,modern', 'MATCH,Choice']\n").unwrap()
    }
    #[test]
    fn automatic_filter_repairs_graph_without_bypassing_a_chain() {
        let input = config();
        let result = apply(
            &input,
            &Policy::default(),
            compatibility::detect(Some("ClashMetaForAndroid/2.10.2.Meta")),
        )
        .unwrap();
        assert_eq!(result.report.removed, 3);
        assert_eq!(result.report.retained, 1);
        assert_eq!(result.config["proxies"][0]["name"], "legacy");
        assert_eq!(result.config["proxy-groups"][1]["proxies"][0], "REJECT");
        assert_eq!(result.config["proxy-groups"][2]["type"], "select");
        assert_eq!(result.config["rules"][0], "DOMAIN,example.com,REJECT");
        assert_eq!(input["proxies"].as_sequence().unwrap().len(), 4);
    }
    #[test]
    fn policies_are_independent_and_unknown_agents_keep_nodes() {
        let input = config();
        let unknown = apply(&input, &Policy::default(), compatibility::detect(None)).unwrap();
        assert_eq!(unknown.config, input);
        assert_eq!(unknown.report.removed, 0);
        let manual = Policy {
            auto: false,
            exclude_types: vec!["anytls".into()],
        };
        let result = apply(
            &input,
            &manual,
            compatibility::detect(Some("ClashMetaForAndroid/2.10.2.Meta")),
        )
        .unwrap();
        assert_eq!(result.report.removed, 1);
        assert_eq!(result.config["proxies"][0]["type"], "mieru");
    }
    #[test]
    fn feature_boundaries_empty_output_and_policy_validation() {
        let mut input = config();
        input["proxies"][0]["transport"] = Value::String("UDP".into());
        let old = apply(
            &input,
            &Policy::default(),
            compatibility::detect(Some("mihomo/1.19.4")),
        )
        .unwrap();
        assert!(
            old.report
                .exclusions
                .iter()
                .any(|e| e.capability == "mieru.udp-transport")
        );
        let new = apply(
            &input,
            &Policy::default(),
            compatibility::detect(Some("mihomo/1.19.17")),
        )
        .unwrap();
        assert_eq!(new.report.removed, 0);
        assert!(
            apply(
                &input,
                &Policy {
                    auto: false,
                    exclude_types: vec![
                        "mieru".into(),
                        "anytls".into(),
                        "ss".into(),
                        "socks5".into()
                    ]
                },
                Detection::default()
            )
            .is_err()
        );
        assert!(Policy::parse(&serde_json::json!({"auto":"false"})).is_err());
        assert!(Policy::parse(&serde_json::json!({"exclude_types":[".*"]})).is_err());
        assert_eq!(
            Policy::parse(&serde_json::json!({"exclude_types":["SS","shadowsocks"]}))
                .unwrap()
                .exclude_types,
            vec!["ss"]
        );
    }
    #[test]
    fn inline_providers_dependencies_rules_and_dns_are_handled_explicitly() {
        let input = crate::engine::parse("proxies: [{name: modern, type: mieru, server: proxy.example, port: 443}, {name: keep, type: ss, server: proxy.example, port: 443}]\nproxy-providers:\n  pool:\n    type: inline\n    payload: [{name: chained, type: socks5, server: proxy.example, port: 1080, dialer-proxy: modern}, {name: nested, type: ss, server: proxy.example, port: 443}]\n  empty:\n    type: inline\n    payload: [{name: modern, type: mieru, server: proxy.example, port: 443}]\nproxy-groups: [{name: Relay, type: relay, use: [pool]}, {name: Empty, type: select, use: [empty]}]\nrules: ['AND,((NETWORK,TCP),(DOMAIN,example.org)),modern', 'IP-CIDR,192.0.2.0/24,modern,no-resolve', 'MATCH,keep']\nsub-rules: {nested: ['MATCH,modern']}\n").unwrap();
        let result = apply(
            &input,
            &Policy::default(),
            compatibility::detect(Some("ClashMetaForAndroid/2.10.2.Meta")),
        )
        .unwrap();
        assert_eq!(
            (
                result.report.before,
                result.report.removed,
                result.report.retained
            ),
            (5, 3, 2)
        );
        assert_eq!(
            result.config["proxy-providers"]["pool"]["payload"]
                .as_sequence()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(result.report.blocked_groups, vec!["Relay", "Empty"]);
        assert_eq!(
            result.config["rules"][0],
            "AND,((NETWORK,TCP),(DOMAIN,example.org)),REJECT"
        );
        assert_eq!(
            result.config["rules"][1],
            "IP-CIDR,192.0.2.0/24,REJECT,no-resolve"
        );
        assert_eq!(result.config["sub-rules"]["nested"][0], "MATCH,REJECT");
        let mut dns = input.clone();
        dns["dns"] =
            serde_yaml::from_str("nameserver: ['https://dns.example/dns-query#modern']").unwrap();
        assert!(
            apply(
                &dns,
                &Policy::default(),
                compatibility::detect(Some("ClashMetaForAndroid/2.10.2.Meta"))
            )
            .is_err()
        );
    }
    #[test]
    fn external_providers_are_never_claimed_to_be_fully_filtered() {
        let mut input = config();
        input["proxy-providers"] = serde_yaml::from_str(
            "remote: {type: http, url: 'https://subscription.example/proxies.yaml', proxy: modern}",
        )
        .unwrap();
        let result = apply(
            &input,
            &Policy::default(),
            compatibility::detect(Some("ClashMetaForAndroid/2.10.2.Meta")),
        )
        .unwrap();
        assert!(!result.report.warnings.is_empty());
        assert_eq!(
            result.config["proxy-providers"]["remote"]["proxy"],
            "REJECT"
        );
        assert!(
            apply(
                &input,
                &Policy {
                    auto: false,
                    exclude_types: vec!["mieru".into()]
                },
                Detection::default()
            )
            .is_err()
        );
        assert!(Policy::parse(&serde_json::json!({"auto":true,"unrecognized":true})).is_err());
    }
}
