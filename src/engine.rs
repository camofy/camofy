//! Stateless configuration composition. No filesystem, network, process or tenant state.
use anyhow::{Result, bail, ensure};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde_yaml::{Mapping, Value};
use std::collections::{BTreeMap, HashSet};

/// Final safety overlay shared by cloud publication and the local runtime boundary.
pub fn cloud_safety_profile(origin: &str) -> Result<Value> {
    let url = url::Url::parse(origin)?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("cloud host missing"))?;
    let rule = match url.host() {
        Some(url::Host::Ipv4(ip)) => format!("IP-CIDR,{ip}/32,DIRECT,no-resolve"),
        Some(url::Host::Ipv6(ip)) => format!("IP-CIDR6,{ip}/128,DIRECT,no-resolve"),
        _ => format!("DOMAIN,{host},DIRECT"),
    };
    Ok(serde_yaml::to_value(
        serde_json::json!({"mode":"rule","prepend-rules":[rule]}),
    )?)
}

#[test]
fn safety_profile_wins_over_global_mode_and_catch_all_rules() {
    for origin in [
        "https://camofy.app",
        "https://custom.example",
        "https://[fd00::1]",
    ] {
        let safety = cloud_safety_profile(origin).unwrap();
        let composed = compose_profiles(&["mode: global\nrules: ['MATCH,REJECT']\nprepend-rules: ['DOMAIN,camofy.app,REJECT']".into(), serde_yaml::to_string(&safety).unwrap()], &BTreeMap::new()).unwrap();
        assert_eq!(composed["mode"], "rule");
        assert_eq!(composed["rules"][0], safety["prepend-rules"][0]);
    }
}

pub fn parse(text: &str) -> Result<Value> {
    ensure!(text.len() <= 4 * 1024 * 1024, "YAML exceeds 4 MiB");
    let value: Value = serde_yaml::from_str(text)?;
    ensure!(value.is_mapping(), "configuration root must be a mapping");
    fn depth(v: &Value, n: usize) -> Result<()> {
        ensure!(n < 64, "YAML nesting exceeds 64 levels");
        match v {
            Value::Mapping(m) => {
                for (k, v) in m {
                    depth(k, n + 1)?;
                    depth(v, n + 1)?;
                }
            }
            Value::Sequence(s) => {
                for v in s {
                    depth(v, n + 1)?;
                }
            }
            Value::Tagged(_) => bail!("YAML tags are unsupported"),
            _ => {}
        }
        Ok(())
    }
    depth(&value, 0)?;
    Ok(value)
}

pub fn merge(base: &mut Value, overlay: &Value) -> Result<()> {
    let dst = base
        .as_mapping_mut()
        .ok_or_else(|| anyhow::anyhow!("base must be mapping"))?;
    let src = overlay
        .as_mapping()
        .ok_or_else(|| anyhow::anyhow!("overlay must be mapping"))?;
    for (k, v) in src {
        if k.as_str()
            .is_some_and(|s| s.starts_with("prepend-") || s.starts_with("append-"))
        {
            let name = k.as_str().unwrap();
            ensure!(
                ["rules", "proxies", "proxy-groups"].contains(&name.split_once('-').unwrap().1),
                "unknown merge directive: {name}"
            );
            continue;
        }
        if let Some(old) = dst.get_mut(k)
            && old.is_mapping()
            && v.is_mapping()
        {
            merge(old, v)?;
            continue;
        }
        dst.insert(k.clone(), v.clone());
    }
    for field in ["rules", "proxies", "proxy-groups"] {
        let pre = src.get(format!("prepend-{field}"));
        let post = src.get(format!("append-{field}"));
        if pre.is_none() && post.is_none() {
            continue;
        }
        let mut result = Vec::new();
        for part in [pre, dst.get(field), post].into_iter().flatten() {
            result.extend(
                part.as_sequence()
                    .ok_or_else(|| anyhow::anyhow!("{field} merge operands must be lists"))?
                    .clone(),
            );
        }
        dst.insert(field.into(), Value::Sequence(result));
    }
    Ok(())
}

pub fn compose(
    source: &str,
    overlays: &[String],
    selections: &BTreeMap<String, String>,
) -> Result<Value> {
    let mut v = parse(source)?;
    for overlay in overlays {
        merge(&mut v, &parse(overlay)?)?;
    }
    select(&mut v, selections)?;
    Ok(v)
}

/// Identity profiles are peers: concatenate rules and upsert named nodes/groups.
/// The six legacy prepend/append directives run after ordinary keys at each step.
pub fn compose_profiles(
    profiles: &[String],
    selections: &BTreeMap<String, String>,
) -> Result<Value> {
    let mut v = Value::Mapping(Mapping::new());
    for text in profiles {
        let profile = if text.trim().is_empty() {
            parse("{}")?
        } else {
            parse(text)?
        };
        let mut ordinary = profile.clone();
        let map = ordinary.as_mapping_mut().unwrap();
        for field in ["rules", "proxies", "proxy-groups"] {
            map.remove(field);
            map.remove(format!("prepend-{field}"));
            map.remove(format!("append-{field}"));
        }
        merge(&mut v, &ordinary)?;
        for field in ["rules", "proxies", "proxy-groups"] {
            for (key, placement) in [
                (field.to_string(), 0),
                (format!("prepend-{field}"), -1),
                (format!("append-{field}"), 1),
            ] {
                let Some(part) = profile.get(&key) else {
                    continue;
                };
                if part.is_null() {
                    if placement == 0 {
                        v.as_mapping_mut()
                            .unwrap()
                            .insert(field.into(), Value::Sequence(vec![]));
                    }
                    continue;
                }
                let incoming = part
                    .as_sequence()
                    .ok_or_else(|| anyhow::anyhow!("{key} must be a list or null"))?;
                let entry = v
                    .as_mapping_mut()
                    .unwrap()
                    .entry(Value::String(field.into()))
                    .or_insert(Value::Sequence(vec![]));
                let items = entry
                    .as_sequence_mut()
                    .ok_or_else(|| anyhow::anyhow!("{field} must be a list"))?;
                if field == "rules" {
                    if placement == -1 {
                        items.splice(0..0, incoming.clone());
                    } else {
                        items.extend(incoming.clone());
                    }
                } else {
                    let mut unique = HashSet::new();
                    for item in incoming {
                        ensure!(
                            unique.insert(required(item, "name")?),
                            "duplicate name within {key}"
                        );
                    }
                    if placement != 0 {
                        items.retain(|item| !unique.contains(item["name"].as_str().unwrap_or("")));
                        if placement == -1 {
                            items.splice(0..0, incoming.clone());
                        } else {
                            items.extend(incoming.clone());
                        }
                    } else {
                        for item in incoming {
                            if let Some(old) =
                                items.iter_mut().find(|old| old["name"] == item["name"])
                            {
                                *old = item.clone();
                            } else {
                                items.push(item.clone());
                            }
                        }
                    }
                }
            }
        }
    }
    select(&mut v, selections)?;
    Ok(v)
}

fn select(v: &mut Value, selections: &BTreeMap<String, String>) -> Result<()> {
    validate(v)?;
    if let Some(groups) = v.get_mut("proxy-groups").and_then(Value::as_sequence_mut) {
        for (name, node) in selections {
            let group = groups
                .iter_mut()
                .find(|g| g["name"].as_str() == Some(name))
                .ok_or_else(|| anyhow::anyhow!("selection group does not exist: {name}"))?;
            ensure!(
                group["type"].as_str() == Some("select"),
                "shared selection requires a select group: {name}"
            );
            let nodes = group
                .get_mut("proxies")
                .and_then(Value::as_sequence_mut)
                .ok_or_else(|| {
                    anyhow::anyhow!("shared selection requires explicit proxies: {name}")
                })?;
            let index = nodes
                .iter()
                .position(|n| n.as_str() == Some(node))
                .ok_or_else(|| anyhow::anyhow!("selected node not in group: {node}"))?;
            let node = nodes.remove(index);
            nodes.insert(0, node);
        }
    } else {
        ensure!(selections.is_empty(), "no groups for shared selections");
    }
    Ok(())
}

pub fn validate(v: &Value) -> Result<()> {
    ensure!(v.is_mapping(), "configuration root must be mapping");
    let mut names: HashSet<String> = [
        "DIRECT",
        "REJECT",
        "REJECT-DROP",
        "PASS",
        "COMPATIBLE",
        "GLOBAL",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    for field in ["proxies", "proxy-groups"] {
        if let Some(list) = v.get(field) {
            for item in list
                .as_sequence()
                .ok_or_else(|| anyhow::anyhow!("{field} must be a list"))?
            {
                let name = required(item, "name")?;
                ensure!(!name.contains([',', '\n', '\r']), "invalid node/group name");
                ensure!(
                    names.insert(name.to_string()),
                    "duplicate node/group name: {name}"
                );
                required(item, "type")?;
                if field == "proxies" {
                    required(item, "server")?;
                    ensure!(
                        item["port"].as_u64().is_some_and(|p| p > 0 && p <= 65535),
                        "invalid proxy port: {name}"
                    );
                }
            }
        }
    }
    if let Some(groups) = v.get("proxy-groups").and_then(Value::as_sequence) {
        let mut edges = BTreeMap::new();
        for group in groups {
            let name = required(group, "name")?;
            let mut refs = Vec::new();
            if let Some(nodes) = group.get("proxies") {
                for n in nodes
                    .as_sequence()
                    .ok_or_else(|| anyhow::anyhow!("group proxies must be list"))?
                {
                    let n = n
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("group references must be strings"))?;
                    ensure!(names.contains(n), "unknown proxy/group reference: {n}");
                    refs.push(n.to_string());
                }
            }
            if let Some(providers) = group.get("use") {
                for p in providers
                    .as_sequence()
                    .ok_or_else(|| anyhow::anyhow!("use must be list"))?
                {
                    ensure!(
                        v.get("proxy-providers").and_then(|x| x.get(p)).is_some(),
                        "unknown proxy provider"
                    );
                }
            }
            edges.insert(name.to_string(), refs);
        }
        fn visit(
            n: &str,
            edges: &BTreeMap<String, Vec<String>>,
            stack: &mut HashSet<String>,
            done: &mut HashSet<String>,
        ) -> Result<()> {
            if done.contains(n) {
                return Ok(());
            }
            ensure!(stack.insert(n.into()), "cyclic proxy group: {n}");
            ensure!(stack.len() <= 64, "proxy group nesting exceeds 64 levels");
            if let Some(next) = edges.get(n) {
                for x in next {
                    visit(x, edges, stack, done)?;
                }
            }
            stack.remove(n);
            done.insert(n.into());
            Ok(())
        }
        let mut done = HashSet::new();
        for n in edges.keys() {
            visit(n, &edges, &mut HashSet::new(), &mut done)?;
        }
    }
    if let Some(rules) = v.get("rules") {
        for rule in rules
            .as_sequence()
            .ok_or_else(|| anyhow::anyhow!("rules must be a list"))?
        {
            let rule = rule
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("rule must be string"))?;
            let parts: Vec<_> = rule.split(',').map(str::trim).collect();
            ensure!(parts.len() >= 2, "invalid rule: {rule}");
            let policy = parts[parts.len() - 1 - usize::from(parts.last() == Some(&"no-resolve"))];
            // Complex logical/sub-rule syntax is passed through to Mihomo.
            if !["AND", "OR", "NOT", "SUB-RULE"].contains(&parts[0]) {
                ensure!(names.contains(policy), "unknown rule policy: {policy}");
            }
        }
    }
    Ok(())
}

fn required<'a>(v: &'a Value, field: &str) -> Result<&'a str> {
    v.get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing string field: {field}"))
}

/// Runtime settings belong to the receiving client. Router defaults are opt-in output.
pub fn mihomo(v: &Value, router: bool) -> Result<String> {
    let mut out = if router {
        let mut defaults = parse(include_str!("router-defaults.yaml"))?;
        merge(&mut defaults, v)?;
        defaults
    } else {
        v.clone()
    };
    let map = out.as_mapping_mut().unwrap();
    for field in [
        "external-controller",
        "external-controller-unix",
        "external-controller-pipe",
        "external-ui",
        "external-ui-url",
        "secret",
    ] {
        map.remove(field);
    }
    if !router {
        for field in [
            "tun",
            "interface-name",
            "routing-mark",
            "mixed-port",
            "port",
            "socks-port",
            "redir-port",
            "tproxy-port",
            "allow-lan",
            "bind-address",
        ] {
            map.remove(field);
        }
        if let Some(dns) = map.get_mut("dns").and_then(Value::as_mapping_mut) {
            dns.remove("listen");
        }
    }
    Ok(serde_yaml::to_string(&out)?)
}

fn sr_nodes(v: &Value) -> Result<&Vec<Value>> {
    ensure!(
        v.get("proxy-providers")
            .and_then(Value::as_mapping)
            .is_none_or(Mapping::is_empty),
        "Shadowrocket export requires inline proxies; proxy-providers are not expanded"
    );
    v.get("proxies")
        .and_then(Value::as_sequence)
        .ok_or_else(|| anyhow::anyhow!("no inline proxies to export"))
}

fn uri(node: &Value) -> Result<String> {
    let kind = required(node, "type")?;
    let allowed = [
        "name",
        "type",
        "server",
        "port",
        "password",
        "cipher",
        "uuid",
        "alterId",
        "tls",
        "servername",
        "sni",
        "network",
        "ws-opts",
        "skip-cert-verify",
        "udp",
    ];
    for k in node.as_mapping().unwrap().keys() {
        ensure!(
            k.as_str().is_some_and(|s| allowed.contains(&s)),
            "unsupported Shadowrocket node option: {:?}",
            k
        );
    }
    let host = required(node, "server")?;
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.into()
    };
    let port = node["port"].as_u64().unwrap();
    let name = required(node, "name")?;
    let network = node["network"].as_str().unwrap_or("tcp");
    ensure!(
        ["tcp", "ws"].contains(&network),
        "unsupported transport: {network}"
    );
    if kind == "vmess" {
        let data = serde_json::json!({"v":"2","ps":name,"add":required(node,"server")?,"port":port.to_string(),"id":required(node,"uuid")?,"aid":node["alterId"].as_u64().unwrap_or(0).to_string(),"scy":node["cipher"].as_str().unwrap_or("auto"),"net":network,"type":"none","host":node["ws-opts"]["headers"]["Host"].as_str().unwrap_or(""),"path":node["ws-opts"]["path"].as_str().unwrap_or("/"),"tls":if node["tls"].as_bool().unwrap_or(false){"tls"}else{""},"sni":node["servername"].as_str().unwrap_or("")});
        ensure!(
            !node["skip-cert-verify"].as_bool().unwrap_or(false),
            "VMess URI does not preserve skip-cert-verify"
        );
        return Ok(format!(
            "vmess://{}",
            STANDARD.encode(serde_json::to_vec(&data)?)
        ));
    }
    ensure!(
        ["ss", "trojan", "vless"].contains(&kind),
        "unsupported Shadowrocket protocol: {kind}"
    );
    let mut url = url::Url::parse(&format!("{kind}://{host}:{port}"))?;
    if kind == "ss" {
        ensure!(
            network == "tcp",
            "SS websocket requires a plugin and cannot be exported"
        );
        let cipher = required(node, "cipher")?;
        let password = required(node, "password")?;
        if cipher.starts_with("2022-") {
            url.set_username(cipher)
                .map_err(|_| anyhow::anyhow!("URI username"))?;
            url.set_password(Some(password))
                .map_err(|_| anyhow::anyhow!("URI password"))?;
        } else {
            url.set_username(&URL_SAFE_NO_PAD.encode(format!("{cipher}:{password}")))
                .map_err(|_| anyhow::anyhow!("URI credentials"))?;
        }
    } else {
        let password = required(node, if kind == "vless" { "uuid" } else { "password" })?;
        url.set_username(password)
            .map_err(|_| anyhow::anyhow!("URI credentials"))?;
        let mut q = url.query_pairs_mut();
        q.append_pair(
            "security",
            if kind == "trojan" || node["tls"].as_bool().unwrap_or(false) {
                "tls"
            } else {
                "none"
            },
        );
        if let Some(sni) = node["sni"].as_str().or(node["servername"].as_str()) {
            q.append_pair("sni", sni);
        }
        if node["skip-cert-verify"].as_bool().unwrap_or(false) {
            q.append_pair("allowInsecure", "1");
        }
        q.append_pair("type", network);
        if network == "ws" {
            q.append_pair("path", node["ws-opts"]["path"].as_str().unwrap_or("/"));
            if let Some(h) = node["ws-opts"]["headers"]["Host"].as_str() {
                q.append_pair("host", h);
            }
        }
    }
    url.set_fragment(Some(name));
    Ok(url.into())
}

pub fn shadowrocket_nodes(v: &Value) -> Result<String> {
    let lines = sr_nodes(v)?.iter().map(uri).collect::<Result<Vec<_>>>()?;
    Ok(STANDARD.encode(lines.join("\n")))
}

/// Shadowrocket's Clash-compatible complete YAML import, with explicit compatibility checks.
/// Native .conf is deliberately not mislabeled as YAML or inferred from user-agent.
pub fn shadowrocket_full(v: &Value) -> Result<String> {
    shadowrocket_nodes(v)?; // Reuse node feature compatibility validation.
    ensure!(
        v.get("rule-providers")
            .and_then(Value::as_mapping)
            .is_none_or(Mapping::is_empty),
        "Shadowrocket full export requires inline rules"
    );
    for g in v["proxy-groups"].as_sequence().into_iter().flatten() {
        ensure!(
            ["select", "url-test", "fallback", "load-balance"].contains(&required(g, "type")?),
            "unsupported Shadowrocket group type"
        );
        ensure!(
            g.get("use").is_none(),
            "Shadowrocket groups require explicit proxies"
        );
    }
    let supported_rules = [
        "DOMAIN",
        "DOMAIN-SUFFIX",
        "DOMAIN-KEYWORD",
        "IP-CIDR",
        "IP-CIDR6",
        "GEOIP",
        "MATCH",
    ];
    for r in v["rules"].as_sequence().into_iter().flatten() {
        ensure!(
            supported_rules.contains(&r.as_str().unwrap().split(',').next().unwrap()),
            "unsupported Shadowrocket rule: {}",
            r.as_str().unwrap()
        );
    }
    let allowed = [
        "proxies",
        "proxy-groups",
        "rules",
        "dns",
        "hosts",
        "mode",
        "ipv6",
        "log-level",
        "allow-lan",
        "port",
        "socks-port",
        "mixed-port",
        "tun",
        "external-controller",
        "external-controller-unix",
        "secret",
        "profile",
    ];
    for k in v.as_mapping().unwrap().keys() {
        ensure!(
            k.as_str().is_some_and(|s| allowed.contains(&s)),
            "unsupported Shadowrocket top-level field: {:?}",
            k
        );
    }
    mihomo(v, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: &str = "proxies:\n- {name: a, type: ss, server: example.com, port: 443, cipher: aes-256-gcm, password: test}\nproxy-groups:\n- {name: pick, type: select, proxies: [a, DIRECT]}\nrules: [MATCH,pick]\n";
    fn source() -> String {
        SOURCE.replace("rules: [MATCH,pick]", "rules: ['MATCH,pick']")
    }
    #[test]
    fn peer_profiles_merge_named_items_rules_and_all_legacy_directives() {
        let out = compose_profiles(&[
            "proxies: [{name: a, type: ss, server: old.example, port: 1}]\nproxy-groups: [{name: pick, type: select, proxies: [a]}]\nrules: ['DOMAIN,first.example,DIRECT']\ndns: {enable: true, nameserver: [a]}".into(),
            "proxies: [{name: a, type: ss, server: new.example, port: 2}, {name: b, type: ss, server: b.example, port: 3}]\nproxy-groups: [{name: pick, type: select, proxies: [b]}]\nrules: ['DOMAIN,second.example,DIRECT']\ndns: {ipv6: false, nameserver: [b]}".into(),
            "prepend-proxies: [{name: b, type: ss, server: pre.example, port: 4}]\nappend-proxies: [{name: c, type: ss, server: c.example, port: 5}]\nprepend-proxy-groups: [{name: before, type: select, proxies: [b]}]\nappend-proxy-groups: [{name: after, type: select, proxies: [c]}]\nprepend-rules: ['DOMAIN,priority.example,before']\nappend-rules: ['MATCH,after']".into()
        ], &BTreeMap::new()).unwrap();
        assert_eq!(out["proxies"][0]["name"], "b");
        assert_eq!(out["proxies"][0]["server"], "pre.example");
        assert_eq!(out["proxies"][1]["server"], "new.example");
        assert_eq!(out["proxies"][2]["name"], "c");
        assert_eq!(out["proxy-groups"][0]["name"], "before");
        assert_eq!(out["proxy-groups"][1]["proxies"][0], "b");
        assert_eq!(out["proxy-groups"][2]["name"], "after");
        assert_eq!(out["rules"].as_sequence().unwrap().len(), 4);
        assert_eq!(out["rules"][0], "DOMAIN,priority.example,before");
        assert_eq!(out["rules"][2], "DOMAIN,second.example,DIRECT");
        assert_eq!(out["rules"][3], "MATCH,after");
        assert_eq!(out["dns"]["enable"], true);
        assert_eq!(out["dns"]["nameserver"], parse("v: [b]").unwrap()["v"]);
        assert!(!serde_yaml::to_string(&out).unwrap().contains("prepend-"));
    }
    #[test]
    fn peer_profile_empty_null_and_invalid_operations() {
        let out = compose_profiles(
            &[
                "".into(),
                "rules: ['MATCH,DIRECT']".into(),
                "rules: null\nprepend-rules: null\nappend-rules: ['MATCH,REJECT']".into(),
            ],
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(out["rules"].as_sequence().unwrap().len(), 1);
        assert_eq!(out["rules"][0], "MATCH,REJECT");
        for bad in [
            "prepend-dns: []",
            "append-proxies: wrong",
            "proxies: [{name: a}, {name: a}]",
            "proxy-groups: [{name: loop, type: select, proxies: [loop]}]",
        ] {
            assert!(
                compose_profiles(&[bad.into()], &BTreeMap::new()).is_err(),
                "{bad}"
            );
        }
    }
    #[test]
    fn ordered_overlays_and_selection() {
        let overlays = vec![
            "prepend-rules: ['DOMAIN,example.com,DIRECT']\ndns: {enable: true}".into(),
            "dns: {ipv6: false}".into(),
        ];
        let v = compose(
            &source(),
            &overlays,
            &BTreeMap::from([("pick".into(), "DIRECT".into())]),
        )
        .unwrap();
        assert_eq!(v["rules"][0].as_str(), Some("DOMAIN,example.com,DIRECT"));
        assert_eq!(v["proxy-groups"][0]["proxies"][0].as_str(), Some("DIRECT"));
        assert_eq!(v["dns"]["enable"].as_bool(), Some(true));
        assert!(v.get("prepend-rules").is_none());
    }
    #[test]
    fn rejects_duplicates_dangling_and_cycles() {
        assert!(
            compose(
                &source(),
                &["append-proxies: [{name: a, type: ss, server: x, port: 1}]".into()],
                &BTreeMap::new()
            )
            .is_err()
        );
        assert!(
            compose(
                &source().replace("[a, DIRECT]", "[missing]"),
                &[],
                &BTreeMap::new()
            )
            .is_err()
        );
        assert!(
            compose(
                &source().replace("[a, DIRECT]", "[pick]"),
                &[],
                &BTreeMap::new()
            )
            .is_err()
        );
    }
    #[test]
    fn outputs_are_distinct_and_unsupported_fails() {
        let v = compose(&source(), &[], &BTreeMap::new()).unwrap();
        let router = parse(&mihomo(&v, true).unwrap()).unwrap();
        assert_eq!(router["tun"]["enable"].as_bool(), Some(true));
        assert!(router.get("external-controller-unix").is_none());
        assert!(
            !mihomo(&v, false)
                .unwrap()
                .contains("external-controller-unix")
        );
        assert!(
            String::from_utf8(STANDARD.decode(shadowrocket_nodes(&v).unwrap()).unwrap())
                .unwrap()
                .starts_with("ss://")
        );
        assert!(shadowrocket_full(&v).unwrap().contains("proxy-groups"));
        let v = compose(
            &source().replace("type: ss", "type: wireguard"),
            &[],
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(shadowrocket_nodes(&v).is_err());
    }
    #[test]
    fn rejects_invalid_roots_and_directives() {
        assert!(parse("- x").is_err());
        assert!(compose(&source(), &["prepend-rules: bad".into()], &BTreeMap::new()).is_err());
    }
    #[test]
    fn router_defaults_never_override_explicit_cloud_profiles() {
        let v = compose(&source(), &["dns: {listen: '127.0.0.1:5353', enhanced-mode: redir-host}\ntun: {enable: false}\nmixed-port: 1234".into()], &BTreeMap::new()).unwrap();
        let out = parse(&mihomo(&v, true).unwrap()).unwrap();
        assert_eq!(out["dns"]["listen"].as_str(), Some("127.0.0.1:5353"));
        assert_eq!(out["tun"]["enable"].as_bool(), Some(false));
        assert_eq!(out["mixed-port"].as_u64(), Some(1234));
        assert!(out["dns"]["nameserver"].is_sequence());
    }
}
