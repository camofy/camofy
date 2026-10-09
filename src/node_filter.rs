//! One graph transformation shared by explicit exclusions and client adaptation.
//! Removed upstreams must never turn a chain into a direct connection.
use crate::compatibility::{self, Detection, Support};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::collections::{BTreeMap, BTreeSet};

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
        if node
            .get("reality-opts")
            .and_then(Value::as_mapping)
            .is_some_and(|opts| !opts.is_empty())
        {
            requirements.push(format!("{kind}.reality"));
            let network = if network.is_empty() { "tcp" } else { network };
            requirements.push(format!("{kind}.reality.{network}"));
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
    output_exclusion: &impl Fn(&Value) -> Option<String>,
) -> Option<(String, String)> {
    let kind = canonical_type(text(node, "type"));
    if policy.exclude_types.contains(&kind) {
        return Some(("manual".into(), kind));
    }
    if let Some(reason) = output_exclusion(node) {
        return Some(("unsupported_output".into(), reason));
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
    output_exclusion: &impl Fn(&Value) -> Option<String>,
) {
    list.retain(|node| {
        report.before += 1;
        if let Some(cause) = reason(node, policy, client, report, output_exclusion) {
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

fn enabled(value: &Value, field: &str) -> bool {
    value.get(field).and_then(Value::as_bool) == Some(true)
}

// Mihomo uses regexp2. Only evaluate the common, linear-time subset here;
// unsupported expressions must not become evidence that a group is nonempty.
fn patterns(value: &Value, field: &str) -> Result<Vec<regex::Regex>> {
    let value = text(value, field);
    if value.is_empty() {
        return Ok(vec![]);
    }
    value
        .split('`')
        .map(|pattern| {
            ensure!(
                pattern.len() <= 16_384
                    && !["\\w", "\\W", "\\s", "\\S", "\\b", "\\B", "&&", "~~"]
                        .iter()
                        .any(|token| pattern.contains(token)),
                "过滤影响了使用特殊正则语义的代理集合或分组，无法验证剩余成员；请使用普通名称匹配或显式节点列表"
            );
            regex::RegexBuilder::new(pattern)
                .size_limit(1 << 20)
                .build()
                .map_err(|_| anyhow::anyhow!("过滤影响了使用不支持的正则表达式的代理集合或分组，无法验证剩余成员；请使用普通名称匹配或显式节点列表"))
        })
        .collect()
}

fn matches_any(patterns: &[regex::Regex], name: &str) -> bool {
    patterns.iter().any(|pattern| pattern.is_match(name))
}

fn provider_node(node: &Value, provider: &Value) -> Value {
    let mut effective = node.clone();
    if !effective.is_mapping() {
        return effective;
    }
    let dialer = text(provider, "dialer-proxy");
    if !dialer.is_empty() {
        effective["dialer-proxy"] = Value::String(dialer.into());
    }
    if let Some(overrides) = provider.get("override") {
        for field in [
            "tfo",
            "mptcp",
            "udp",
            "udp-over-tcp",
            "up",
            "down",
            "dialer-proxy",
            "skip-cert-verify",
            "interface-name",
            "routing-mark",
            "ip-version",
        ] {
            if let Some(value) = overrides.get(field).filter(|value| !value.is_null()) {
                effective[field] = value.clone();
            }
        }
        let name = format!(
            "{}{}{}",
            text(overrides, "additional-prefix"),
            text(node, "name"),
            text(overrides, "additional-suffix")
        );
        effective["name"] = Value::String(name);
    }
    effective
}

fn inline_members(provider: &Value) -> Result<Vec<Value>> {
    let include = patterns(provider, "filter")?;
    let exclude = patterns(provider, "exclude-filter")?;
    let exclude_types: Vec<_> = text(provider, "exclude-type").split('|').collect();
    ensure!(
        provider
            .get("override")
            .and_then(|v| v.get("proxy-name"))
            .is_none_or(|value| value.is_null() || value.as_sequence().is_some_and(Vec::is_empty)),
        "过滤影响了使用正则重命名的代理集合，无法验证剩余成员；请先将节点作为普通订阅源导入"
    );
    Ok(provider
        .get("payload")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter(|node| {
            let name = text(node, "name");
            (include.is_empty() || matches_any(&include, name))
                && !matches_any(&exclude, name)
                && !exclude_types
                    .iter()
                    .any(|kind| kind.eq_ignore_ascii_case(text(node, "type")))
        })
        .map(|node| provider_node(node, provider))
        .collect())
}

// exclude-type uses AdapterType.String(), unlike provider exclude-type, which
// compares the serialized protocol identifier before parsing the node.
fn adapter_type(kind: &str) -> &str {
    match kind {
        "ss" => "Shadowsocks",
        "ssr" => "ShadowsocksR",
        "select" => "Selector",
        "url-test" => "URLTest",
        "load-balance" => "LoadBalance",
        "reject-drop" => "RejectDrop",
        other => other,
    }
}

#[derive(Default)]
struct Members {
    names: BTreeSet<String>,
    remote: bool,
}

fn group_members(group: &Value, config: &Value) -> Result<Members> {
    let include = patterns(group, "filter")?;
    let exclude = patterns(group, "exclude-filter")?;
    let exclude_types: Vec<_> = text(group, "exclude-type").split('|').collect();
    let accepted = |name: &str, kind: &str| {
        !matches_any(&exclude, name)
            && !exclude_types
                .iter()
                .any(|excluded| excluded.eq_ignore_ascii_case(adapter_type(kind)))
    };
    let mut types: BTreeMap<String, String> = [
        ("DIRECT", "direct"),
        ("REJECT", "reject"),
        ("REJECT-DROP", "reject-drop"),
        ("PASS", "pass"),
        ("COMPATIBLE", "compatible"),
        ("GLOBAL", "select"),
    ]
    .into_iter()
    .map(|(name, kind)| (name.into(), kind.into()))
    .collect();
    for field in ["proxies", "proxy-groups"] {
        for node in config
            .get(field)
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
        {
            types.insert(text(node, "name").into(), text(node, "type").into());
        }
    }
    let mut result = Members::default();
    // Group filter does not apply to explicitly listed proxies in Mihomo.
    for name in group
        .get("proxies")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if types.get(name).is_some_and(|kind| accepted(name, kind)) {
            result.names.insert(name.into());
        }
    }
    if enabled(group, "include-all") || enabled(group, "include-all-proxies") {
        for node in config
            .get("proxies")
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
        {
            let name = text(node, "name");
            if (include.is_empty() || matches_any(&include, name))
                && accepted(name, text(node, "type"))
            {
                result.names.insert(name.into());
            }
        }
    }
    if let Some(providers) = config.get("proxy-providers").and_then(Value::as_mapping) {
        let all = enabled(group, "include-all") || enabled(group, "include-all-providers");
        let used: BTreeSet<_> = group
            .get("use")
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        for (name, provider) in providers {
            if !all && !name.as_str().is_some_and(|name| used.contains(name)) {
                continue;
            }
            if text(provider, "type") != "inline" {
                result.remote = true;
                continue;
            }
            for node in inline_members(provider)? {
                let name = text(&node, "name");
                if (include.is_empty() || matches_any(&include, name))
                    && accepted(name, text(&node, "type"))
                {
                    result.names.insert(name.into());
                }
            }
        }
    }
    Ok(result)
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
    apply_with_exclusions(input, policy, client, |_| None)
}

/// Additional output restrictions are separate from client capabilities. The
/// predicate sees effective inline-provider options and shares graph repair.
pub fn apply_with_exclusions(
    input: &Value,
    policy: &Policy,
    client: Detection,
    output_exclusion: impl Fn(&Value) -> Option<String>,
) -> Result<Filtered> {
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
        prune(
            nodes,
            policy,
            &client,
            &mut report,
            &mut removed,
            &output_exclusion,
        );
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
                let options = provider.clone();
                if let Some(nodes) = provider.get_mut("payload").and_then(Value::as_sequence_mut) {
                    let before = nodes.len();
                    nodes.retain(|node| {
                        report.before += 1;
                        let effective = provider_node(node, &options);
                        if let Some(cause) =
                            reason(&effective, policy, &client, &mut report, &output_exclusion)
                        {
                            record(&effective, cause, &mut report);
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
            let options = provider.clone();
            if text(provider, "type") == "inline"
                && let Some(nodes) = provider.get_mut("payload").and_then(Value::as_sequence_mut)
            {
                let before = nodes.len();
                nodes.retain(|node| {
                    let effective = provider_node(node, &options);
                    if removed.contains(text(&effective, "dialer-proxy")) {
                        record(
                            &effective,
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
            } else if text(provider, "type") != "inline" {
                // A provider-wide dialer applies to nodes fetched later too.
                let effective = provider_node(
                    &serde_yaml::from_str("{name: placeholder}").unwrap(),
                    provider,
                );
                if removed.contains(text(&effective, "dialer-proxy")) {
                    empty_providers.insert(name.as_str().unwrap_or("").to_owned());
                    changed_providers.insert(name.as_str().unwrap_or("").to_owned());
                    remote_providers -= 1;
                    report
                        .warnings
                        .push("已移除依赖被排除出口的远程代理集合，以免留下失效链路。".into());
                }
            }
        }
    }
    // Provider filters are evaluated before node overrides by Mihomo. A partial
    // protocol deletion can therefore empty a provider even with raw payload left.
    if let Some(providers) = config
        .get_mut("proxy-providers")
        .and_then(Value::as_mapping_mut)
    {
        for (name, provider) in providers.iter_mut() {
            let name = name.as_str().unwrap_or("");
            if text(provider, "type") != "inline" || empty_providers.contains(name) {
                continue;
            }
            let empty = match inline_members(provider) {
                Ok(members) => members.is_empty(),
                Err(error) if changed_providers.contains(name) => return Err(error),
                Err(_) => {
                    let warning =
                        "部分代理集合的筛选或重命名语义尚未验证，已保留原设置。".to_owned();
                    if !report.warnings.contains(&warning) {
                        report.warnings.push(warning);
                    }
                    false
                }
            };
            if empty {
                for node in provider
                    .get("payload")
                    .and_then(Value::as_sequence)
                    .into_iter()
                    .flatten()
                {
                    record(
                        node,
                        ("dependency".into(), "provider_selection".into()),
                        &mut report,
                    );
                }
                empty_providers.insert(name.into());
            }
        }
        // Unused inline providers are still parsed by the core; an empty payload
        // makes the entire configuration invalid, even after all use refs vanish.
        providers.retain(|name, _| {
            !name
                .as_str()
                .is_some_and(|name| empty_providers.contains(name))
        });
    }
    changed_providers.extend(empty_providers.iter().cloned());
    let mut affected_groups = BTreeSet::new();
    if let Some(groups) = config
        .get_mut("proxy-groups")
        .and_then(Value::as_sequence_mut)
    {
        for group in groups {
            let all = enabled(group, "include-all");
            let mut affected = (all || enabled(group, "include-all-proxies"))
                && !removed.is_empty()
                || (all || enabled(group, "include-all-providers"))
                    && !changed_providers.is_empty();
            if let Some(members) = group.get_mut("proxies").and_then(Value::as_sequence_mut) {
                let before = members.len();
                members.retain(|v| !v.as_str().is_some_and(|name| removed.contains(name)));
                affected |= before != members.len();
                report.repaired_references += before - members.len();
            }
            if let Some(providers) = group.get_mut("use").and_then(Value::as_sequence_mut) {
                affected |= providers.iter().any(|v| {
                    v.as_str()
                        .is_some_and(|name| changed_providers.contains(name))
                });
                let before = providers.len();
                providers.retain(|v| {
                    !v.as_str()
                        .is_some_and(|name| empty_providers.contains(name))
                });
                report.repaired_references += before - providers.len();
            }
            if affected {
                affected_groups.insert(text(group, "name").to_owned());
            }
        }
    }
    // Blocking a group changes its adapter type. A parent's exclude-type can
    // consequently become empty as well, so evaluate the graph to a fixed point.
    let mut blocked = BTreeSet::new();
    for _ in 0..=64 {
        let snapshot = config.clone();
        let previous = blocked.len();
        if let Some(groups) = config
            .get_mut("proxy-groups")
            .and_then(Value::as_sequence_mut)
        {
            for group in groups {
                let name = text(group, "name").to_owned();
                if blocked.contains(&name) {
                    continue;
                }
                let affected = affected_groups.contains(&name)
                    || group
                        .get("proxies")
                        .and_then(Value::as_sequence)
                        .into_iter()
                        .flatten()
                        .any(|v| v.as_str().is_some_and(|name| blocked.contains(name)));
                if affected && text(group, "type") == "relay" {
                    block_group(group, &mut report);
                    blocked.insert(name);
                    continue;
                }
                let members = match group_members(group, &snapshot) {
                    Ok(members) => members,
                    Err(error) if affected => return Err(error),
                    Err(_) => {
                        let warning = "部分代理组的特殊筛选语义尚未验证，已保留原设置。".to_owned();
                        if !report.warnings.contains(&warning) {
                            report.warnings.push(warning);
                        }
                        continue;
                    }
                };
                if members.names.is_empty() {
                    if members.remote {
                        ensure!(
                            !affected,
                            "过滤后分组只剩未展开的远程代理集合，无法验证是否会回退为直连；请先将节点作为普通订阅源导入"
                        );
                        let warning =
                            "部分代理组只引用未展开的远程代理集合，其空集合回退行为尚未验证。"
                                .to_owned();
                        if !report.warnings.contains(&warning) {
                            report.warnings.push(warning);
                        }
                        continue;
                    }
                    block_group(group, &mut report);
                    blocked.insert(name);
                } else if present(group, "default-selected")
                    && !members.remote
                    && !members.names.contains(text(group, "default-selected"))
                {
                    group.as_mapping_mut().unwrap().remove("default-selected");
                    report.repaired_references += 1;
                }
            }
        }
        if blocked.len() == previous {
            break;
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

    fn graph_fixture(extra: &str) -> Value {
        crate::engine::parse(&format!(
            "proxies:\n- {{name: modern, type: mieru, server: proxy.example, port: 443}}\n- {{name: legacy, type: ss, server: proxy.example, port: 443}}\n{extra}"
        )).unwrap()
    }

    fn exclude_modern(input: &Value) -> Result<Filtered> {
        apply(
            input,
            &Policy {
                auto: false,
                exclude_types: vec!["mieru".into()],
            },
            Detection::default(),
        )
    }

    #[test]
    fn effective_empty_groups_are_blocked_instead_of_core_direct_fallback() {
        for group in [
            "{name: Choice, type: select, include-all-proxies: true, filter: '^modern$'}",
            "{name: Choice, type: select, include-all: true, filter: '^modern$'}",
            "{name: Choice, type: select, proxies: [modern, legacy], exclude-filter: '^legacy$'}",
            "{name: Choice, type: select, proxies: [modern, legacy], exclude-type: Shadowsocks}",
        ] {
            let input = graph_fixture(&format!(
                "proxy-groups: [{group}]\nrules: ['MATCH,Choice']\n"
            ));
            let result = exclude_modern(&input).unwrap();
            assert_eq!(result.report.blocked_groups, ["Choice"], "{group}");
            assert_eq!(result.config["proxy-groups"][0]["proxies"][0], "REJECT");
            assert!(
                result.config["proxy-groups"][0]
                    .get("include-all")
                    .is_none()
            );
        }
    }

    #[test]
    fn group_filter_skips_explicit_members_and_stale_selection_is_removed() {
        let input = graph_fixture(
            "proxy-groups: [{name: Choice, type: select, proxies: [modern, legacy], filter: '^modern$', default-selected: modern}]\nrules: ['MATCH,Choice']\n",
        );
        let result = exclude_modern(&input).unwrap();
        assert!(result.report.blocked_groups.is_empty());
        assert_eq!(result.config["proxy-groups"][0]["proxies"][0], "legacy");
        assert!(
            result.config["proxy-groups"][0]
                .get("default-selected")
                .is_none()
        );
    }

    #[test]
    fn empty_inline_provider_definitions_and_dynamic_uses_are_removed() {
        for inclusion in [
            "use: [pool]",
            "include-all-providers: true",
            "include-all: true, filter: '^only$'",
        ] {
            let input = graph_fixture(&format!(
                "proxy-providers: {{pool: {{type: inline, payload: [{{name: only, type: mieru}}]}}}}\nproxy-groups: [{{name: Choice, type: select, {inclusion}}}]\nrules: ['MATCH,Choice']\n"
            ));
            let result = exclude_modern(&input).unwrap();
            assert!(
                result.config["proxy-providers"]
                    .as_mapping()
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(result.report.blocked_groups, ["Choice"]);
            assert_eq!(result.report.retained, 1);
        }
    }

    #[test]
    fn provider_filters_and_name_overrides_affect_effective_members() {
        let input = graph_fixture(
            "proxy-providers:\n  pool:\n    type: inline\n    payload: [{name: first, type: mieru}, {name: second, type: ss}]\n    override: {additional-prefix: 'prefix-'}\nproxy-groups: [{name: Choice, type: select, use: [pool], filter: '^prefix-first$'}]\nrules: ['MATCH,Choice']\n",
        );
        let result = exclude_modern(&input).unwrap();
        assert_eq!(result.report.blocked_groups, ["Choice"]);
        assert_eq!(
            result.config["proxy-providers"]["pool"]["payload"]
                .as_sequence()
                .unwrap()
                .len(),
            1
        );

        let mut filtered_provider = input;
        filtered_provider["proxy-providers"]["pool"]["filter"] = "^first$".into();
        let result = exclude_modern(&filtered_provider).unwrap();
        assert!(
            result.config["proxy-providers"]
                .as_mapping()
                .unwrap()
                .is_empty()
        );
        assert_eq!(result.report.retained, 1);
    }

    #[test]
    fn provider_dialer_overrides_follow_actual_override_precedence() {
        for options in ["dialer-proxy: modern", "override: {dialer-proxy: modern}"] {
            let input = graph_fixture(&format!(
                "proxy-providers: {{pool: {{type: inline, {options}, payload: [{{name: member, type: ss, dialer-proxy: legacy}}]}}}}\nproxy-groups: [{{name: Choice, type: select, use: [pool]}}]\nrules: ['MATCH,Choice']\n"
            ));
            let result = exclude_modern(&input).unwrap();
            assert!(
                result.config["proxy-providers"]
                    .as_mapping()
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(result.report.blocked_groups, ["Choice"]);
            assert_eq!(result.report.removed, 2);
        }
        let input = graph_fixture(
            "proxy-providers: {pool: {type: inline, dialer-proxy: modern, override: {dialer-proxy: legacy}, payload: [{name: member, type: ss}]}}\nproxy-groups: [{name: Choice, type: select, use: [pool]}]\nrules: ['MATCH,Choice']\n",
        );
        let result = exclude_modern(&input).unwrap();
        assert!(result.report.blocked_groups.is_empty());
        assert_eq!(result.report.removed, 1);
    }

    #[test]
    fn output_limitations_share_dependency_repair_and_effective_options() {
        let input = graph_fixture(
            "proxy-providers: {pool: {type: inline, override: {udp: true}, payload: [{name: legacy, type: ss}]}}\nproxy-groups: [{name: Choice, type: select, use: [pool]}]\nrules: ['MATCH,Choice']\n",
        );
        let result =
            apply_with_exclusions(&input, &Policy::default(), Detection::default(), |node| {
                enabled(node, "udp").then(|| "output cannot preserve UDP".into())
            })
            .unwrap();
        assert_eq!(result.report.removed, 1);
        assert_eq!(result.report.exclusions[0].reason, "unsupported_output");
        assert_eq!(result.config["proxies"].as_sequence().unwrap().len(), 2);
        assert!(
            result.config["proxy-providers"]
                .as_mapping()
                .unwrap()
                .is_empty()
        );
        assert_eq!(result.report.blocked_groups, ["Choice"]);
    }

    #[test]
    fn affected_unverifiable_regex_fails_but_unaffected_group_is_unchanged() {
        let affected = graph_fixture(
            "proxy-groups: [{name: Choice, type: select, include-all-proxies: true, filter: '^(?!legacy)'}]\nrules: ['MATCH,Choice']\n",
        );
        assert!(exclude_modern(&affected).is_err());
        let unaffected = graph_fixture(
            "proxy-groups: [{name: Choice, type: select, proxies: [legacy], filter: '^(?!legacy)'}]\nrules: ['MATCH,Choice']\n",
        );
        let result = exclude_modern(&unaffected).unwrap();
        assert_eq!(result.config["proxy-groups"], unaffected["proxy-groups"]);
    }

    #[test]
    fn parent_type_exclusions_are_rechecked_after_child_is_blocked() {
        let input = graph_fixture(
            "proxy-groups:\n- {name: Parent, type: select, proxies: [Child], exclude-type: Selector}\n- {name: Child, type: url-test, proxies: [modern]}\nrules: ['MATCH,Parent']\n",
        );
        let result = exclude_modern(&input).unwrap();
        assert_eq!(result.report.blocked_groups, ["Child", "Parent"]);
        assert_eq!(result.config["proxy-groups"][0]["proxies"][0], "REJECT");
    }

    #[test]
    fn reality_requirements_include_transport_only_for_nonempty_options() {
        let node = crate::engine::parse("type: vless\nreality-opts: {}\n").unwrap();
        assert!(
            !requirements(&node, "vless")
                .iter()
                .any(|key| key.contains("reality"))
        );
        let mut node = node;
        node["reality-opts"]["public-key"] = "example-public-key".into();
        assert!(requirements(&node, "vless").contains(&"vless.reality.tcp".into()));
        node["network"] = "ws".into();
        assert!(requirements(&node, "vless").contains(&"vless.reality.ws".into()));
    }

    #[test]
    fn legacy_empty_groups_are_repaired_even_without_new_exclusions() {
        let input = crate::engine::parse("proxies: [{name: legacy, type: ss, server: proxy.example, port: 443}]\nproxy-groups: [{name: Choice, type: select, include-all-proxies: true, filter: '^modern$'}]\nrules: ['MATCH,Choice']\n").unwrap();
        let result = apply(
            &input,
            &Policy {
                auto: false,
                exclude_types: vec![],
            },
            Detection::default(),
        )
        .unwrap();
        assert_eq!(result.report.removed, 0);
        assert_eq!(result.report.blocked_groups, ["Choice"]);
        assert_eq!(result.config["proxy-groups"][0]["proxies"][0], "REJECT");
        let direct_only = crate::engine::parse("rules: ['MATCH,DIRECT']\n").unwrap();
        assert_eq!(exclude_modern(&direct_only).unwrap().config, direct_only);
    }
}
