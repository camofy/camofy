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
        _ if host == "camofy.app" => format!("DOMAIN-SUFFIX,{host},DIRECT"),
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
        let expected = match origin {
            "https://camofy.app" => "DOMAIN-SUFFIX,camofy.app,DIRECT",
            "https://custom.example" => "DOMAIN,custom.example,DIRECT",
            _ => "IP-CIDR6,fd00::1/128,DIRECT,no-resolve",
        };
        assert_eq!(safety["prepend-rules"][0], expected);
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
        if k.as_str() == Some("proxy-group-patches") {
            continue;
        }
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
    apply_proxy_group_patches(base, src.get("proxy-group-patches"))?;
    Ok(())
}

/// Apply group-scoped changes after node and group definitions in the same profile.
/// A patch never replaces a group's other settings or appears in rendered YAML.
fn apply_proxy_group_patches(config: &mut Value, patches: Option<&Value>) -> Result<()> {
    let Some(patches) = patches.filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let patches = patches
        .as_sequence()
        .ok_or_else(|| anyhow::anyhow!("proxy-group-patches must be a list or null"))?;
    let mut patched_groups = HashSet::new();
    for patch in patches {
        let fields = patch
            .as_mapping()
            .ok_or_else(|| anyhow::anyhow!("proxy-group-patches entries must be mappings"))?;
        ensure!(
            fields
                .keys()
                .all(|key| matches!(key.as_str(), Some("name" | "append-proxies"))),
            "unknown proxy-group-patches field"
        );
        let name = required(patch, "name")?;
        ensure!(
            patched_groups.insert(name),
            "duplicate proxy-group-patches target: {name}"
        );
        let additions = patch
            .get("append-proxies")
            .and_then(Value::as_sequence)
            .ok_or_else(|| {
                anyhow::anyhow!("proxy-group-patches append-proxies must be a list: {name}")
            })?;
        let mut unique = HashSet::new();
        for addition in additions {
            let member = addition
                .as_str()
                .filter(|member| !member.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!("proxy-group-patches members must be nonempty strings: {name}")
                })?;
            ensure!(
                unique.insert(member),
                "duplicate proxy-group-patches member in {name}: {member}"
            );
        }
        let group = config
            .get_mut("proxy-groups")
            .and_then(Value::as_sequence_mut)
            .and_then(|groups| {
                groups
                    .iter_mut()
                    .find(|group| group["name"].as_str() == Some(name))
            })
            .ok_or_else(|| anyhow::anyhow!("proxy-group-patches target does not exist: {name}"))?;
        let members = group
            .as_mapping_mut()
            .ok_or_else(|| anyhow::anyhow!("proxy group must be a mapping: {name}"))?
            .entry(Value::String("proxies".into()))
            .or_insert_with(|| Value::Sequence(Vec::new()))
            .as_sequence_mut()
            .ok_or_else(|| anyhow::anyhow!("proxy group proxies must be a list: {name}"))?;
        for addition in additions {
            if !members.contains(addition) {
                members.push(addition.clone());
            }
        }
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
/// The six legacy prepend/append directives and group patches run after ordinary keys.
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
        map.remove("proxy-group-patches");
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
        apply_proxy_group_patches(&mut v, profile.get("proxy-group-patches"))?;
    }
    select(&mut v, selections)?;
    Ok(v)
}

/// Export defaults are preferences, not runtime state. Stale selections must not
/// prevent a valid upstream configuration from publishing.
pub fn selection_defaults(v: &mut Value, selections: &BTreeMap<String, String>) {
    if let Some(groups) = v.get_mut("proxy-groups").and_then(Value::as_sequence_mut) {
        for group in groups {
            if group["type"].as_str() != Some("select") {
                continue;
            }
            let Some(node) = group["name"].as_str().and_then(|name| selections.get(name)) else {
                continue;
            };
            group
                .as_mapping_mut()
                .unwrap()
                .insert("default-selected".into(), node.clone().into());
            if let Some(nodes) = group.get_mut("proxies").and_then(Value::as_sequence_mut)
                && let Some(index) = nodes.iter().position(|n| n.as_str() == Some(node))
            {
                let selected = nodes.remove(index);
                nodes.insert(0, selected);
            }
        }
    }
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
    {
        let mut edges = BTreeMap::new();
        if let Some(proxies) = v.get("proxies").and_then(Value::as_sequence) {
            for node in proxies {
                if let Some(dialer) = node.get("dialer-proxy") {
                    let target = dialer
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("dialer-proxy must be a name"))?;
                    ensure!(names.contains(target), "unknown dialer-proxy: {target}");
                    edges.insert(required(node, "name")?.to_owned(), vec![target.to_owned()]);
                }
            }
        }
        if let Some(groups) = v.get("proxy-groups").and_then(Value::as_sequence) {
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
            if parts[0] == "RULE-SET" {
                ensure!(
                    parts.len() >= 3
                        && v.get("rule-providers")
                            .and_then(|p| p.get(parts[1]))
                            .is_some(),
                    "unknown rule provider: {}",
                    parts[1]
                );
            }
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

#[test]
fn dialer_proxy_cycle_through_group_is_rejected() {
    let yaml = r#"
proxies:
  - { name: Relay, type: socks5, server: example.org, port: 1080, dialer-proxy: Route }
proxy-groups:
  - { name: Route, type: select, proxies: [Relay] }
"#;
    let value = parse(yaml).unwrap();
    assert!(validate(&value).unwrap_err().to_string().contains("cyclic"));
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

// These checks describe this encoder, not the receiving application's capabilities.
// Errors must never include field values: credentials and server addresses are private.
fn uri_fields(v: &Value, allowed: &[&str], path: &str) -> Result<()> {
    let fields = v
        .as_mapping()
        .ok_or_else(|| anyhow::anyhow!("invalid URI input: {path} must be a mapping"))?;
    for (key, value) in fields {
        // Generic subscription converters emit null placeholders for fields that
        // do not apply to this protocol. Null has no setting to preserve; false,
        // zero and empty strings still require protocol-specific handling.
        if value.is_null() {
            continue;
        }
        if !key.as_str().is_some_and(|s| allowed.contains(&s)) {
            let field = diagnostic_key(key.as_str());
            bail!("URI exporter cannot preserve {path}.{field}");
        }
    }
    Ok(())
}

fn diagnostic_key(key: Option<&str>) -> &str {
    key.filter(|s| s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .unwrap_or("unknown-field")
}

fn uri_string<'a>(v: &'a Value, field: &str) -> Result<Option<&'a str>> {
    match v.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        _ => bail!("invalid URI input: {field} must be a string"),
    }
}

fn uri_bool(v: &Value, field: &str) -> Result<Option<bool>> {
    match v.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        _ => bail!("invalid URI input: {field} must be a boolean"),
    }
}

fn uri_sni(node: &Value) -> Result<Option<&str>> {
    let sni = uri_string(node, "sni")?;
    let servername = uri_string(node, "servername")?;
    ensure!(
        sni.is_none() || servername.is_none() || sni == servername,
        "invalid URI input: conflicting sni and servername"
    );
    Ok(sni.or(servername).filter(|s| !s.is_empty()))
}

fn uri_alpn(node: &Value) -> Result<Option<String>> {
    match node.get("alpn") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Sequence(items)) => {
            ensure!(
                !items.is_empty(),
                "invalid URI input: alpn must not be empty"
            );
            let values = items
                .iter()
                .map(|item| {
                    item.as_str()
                        .filter(|s| {
                            !s.is_empty() && !s.contains(',') && !s.chars().any(char::is_control)
                        })
                        .ok_or_else(|| {
                            anyhow::anyhow!("invalid URI input: alpn must contain protocol strings")
                        })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Some(values.join(",")))
        }
        _ => bail!("invalid URI input: alpn must be a list"),
    }
}

fn uri_endpoint(node: &Value, scheme: &str) -> Result<url::Url> {
    let server = required(node, "server")?;
    ensure!(
        !server.chars().any(char::is_whitespace)
            && !server.contains(['/', '?', '#', '@', '\\', '%']),
        "invalid URI input: server must be a host or IP address"
    );
    let host = if let Ok(ip) = server.parse::<std::net::Ipv6Addr>() {
        format!("[{ip}]")
    } else {
        ensure!(
            !server.contains([':', '[', ']']),
            "invalid URI input: server must be a host or IP address"
        );
        server.to_string()
    };
    let port = node["port"]
        .as_u64()
        .filter(|p| *p > 0 && *p <= 65535)
        .ok_or_else(|| anyhow::anyhow!("invalid URI input: port must be between 1 and 65535"))?;
    url::Url::parse(&format!("{scheme}://{host}:{port}"))
        .map_err(|_| anyhow::anyhow!("invalid URI input: server or port"))
}

fn uri_credentials(url: &mut url::Url, username: &str, password: Option<&str>) -> Result<()> {
    // Url setters preserve existing percent escapes. Encode literal '%' too, so
    // an account containing "%2F" is not changed to "/" by the receiving client.
    const USERINFO: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    let username = percent_encoding::utf8_percent_encode(username, USERINFO).to_string();
    let password = password.map(|s| percent_encoding::utf8_percent_encode(s, USERINFO).to_string());
    url.set_username(&username)
        .map_err(|_| anyhow::anyhow!("URI exporter cannot encode username"))?;
    url.set_password(password.as_deref())
        .map_err(|_| anyhow::anyhow!("URI exporter cannot encode password"))?;
    Ok(())
}

fn uri_ws(node: &Value, network: &str) -> Result<(String, Option<String>)> {
    let Some(ws) = node.get("ws-opts").filter(|v| !v.is_null()) else {
        return Ok(("/".into(), None));
    };
    uri_fields(
        ws,
        &[
            "path",
            "headers",
            "max-early-data",
            "early-data-header-name",
            "v2ray-http-upgrade",
            "v2ray-http-upgrade-fast-open",
        ],
        "ws-opts",
    )?;
    ensure!(
        network == "ws" || ws.as_mapping().is_some_and(Mapping::is_empty),
        "URI exporter cannot preserve ws-opts without network ws"
    );
    // Do not turn HTTP Upgrade into WebSocket or silently strip early-data/header
    // requirements. Their client-specific URI dialects need separate evidence.
    for key in ["v2ray-http-upgrade", "v2ray-http-upgrade-fast-open"] {
        ensure!(
            uri_bool(ws, key)? != Some(true),
            "URI exporter cannot preserve ws-opts.{key}"
        );
    }
    if let Some(n) = ws.get("max-early-data").filter(|v| !v.is_null()) {
        ensure!(
            n.as_u64().is_some(),
            "invalid URI input: ws-opts.max-early-data must be a nonnegative integer"
        );
        ensure!(
            n.as_u64() == Some(0),
            "URI exporter cannot preserve ws-opts.max-early-data"
        );
    }
    ensure!(
        uri_string(ws, "early-data-header-name")?.is_none_or(str::is_empty),
        "URI exporter cannot preserve ws-opts.early-data-header-name"
    );
    let path = uri_string(ws, "path")?.unwrap_or("/");
    ensure!(
        !path.is_empty(),
        "invalid URI input: ws-opts.path must not be empty"
    );
    let mut host = None;
    if let Some(headers) = ws.get("headers").filter(|v| !v.is_null()) {
        let headers = headers.as_mapping().ok_or_else(|| {
            anyhow::anyhow!("invalid URI input: ws-opts.headers must be a mapping")
        })?;
        for (name, value) in headers {
            ensure!(
                name.as_str()
                    .is_some_and(|s| s.eq_ignore_ascii_case("host")),
                "URI exporter cannot preserve ws-opts.headers other than Host"
            );
            ensure!(
                host.is_none(),
                "invalid URI input: duplicate ws-opts.headers Host"
            );
            host = Some(
                value
                    .as_str()
                    .ok_or_else(|| {
                        anyhow::anyhow!("invalid URI input: ws-opts.headers.Host must be a string")
                    })?
                    .to_string(),
            );
        }
    }
    Ok((path.to_string(), host))
}

/// Encode one node without discarding connection parameters. Callers may use the
/// error as an exportability diagnostic independently of client/version support.
/// Sharing local routing preferences is not part of a URI: an explicit disabled
/// UDP capability is rejected rather than silently enabling it on another client.
///
/// Wire formats: SIP002; v2rayN's VMess share-link specification; XTLS discussion
/// #716; Hysteria's URI-Scheme; enfein/mieru pkg/appctl/url.go (mierus).
pub fn shadowrocket_node_uri(node: &Value) -> Result<String> {
    let kind = required(node, "type")?;
    let common = ["name", "type", "server", "port", "udp"];
    let extra: &[&str] = match kind {
        "ss" => &["cipher", "password"],
        "socks5" | "http" => &["username", "password", "tls"],
        "vmess" => &[
            "uuid",
            "alterId",
            "cipher",
            "tls",
            "servername",
            "sni",
            "network",
            "ws-opts",
            "skip-cert-verify",
            "alpn",
            "client-fingerprint",
        ],
        "vless" => &[
            "uuid",
            "alterId",
            "cipher",
            "tls",
            "servername",
            "sni",
            "network",
            "ws-opts",
            "skip-cert-verify",
            "alpn",
            "client-fingerprint",
            "reality-opts",
            "flow",
            "encryption",
        ],
        "trojan" => &[
            "password",
            "tls",
            "servername",
            "sni",
            "network",
            "ws-opts",
            "skip-cert-verify",
            "alpn",
            "client-fingerprint",
        ],
        "hysteria2" => &[
            "password",
            "ports",
            "hop-interval",
            "sni",
            "skip-cert-verify",
            "obfs",
            "obfs-password",
            "fingerprint",
        ],
        "mieru" => &[
            "username",
            "password",
            "transport",
            "multiplexing",
            "handshake-mode",
        ],
        _ => bail!("URI exporter has not implemented this node protocol"),
    };
    let allowed = common
        .into_iter()
        .chain(extra.iter().copied())
        .collect::<Vec<_>>();
    uri_fields(node, &allowed, "node")?;
    if kind == "vless" {
        // Mihomo v1.19.17 adapter/outbound/vless.go: VlessOption has neither
        // alterId nor cipher. Accept only conventional VMess default placeholders
        // emitted by generic converters, never an arbitrary cross-protocol value.
        if let Some(alter_id) = node.get("alterId").filter(|v| !v.is_null()) {
            ensure!(
                alter_id.as_u64() == Some(0),
                "URI exporter cannot preserve node.alterId for VLESS except the zero placeholder"
            );
        }
        ensure!(
            matches!(uri_string(node, "cipher")?, None | Some("auto" | "none")),
            "URI exporter cannot preserve node.cipher for VLESS except auto or none placeholders"
        );
    } else if kind == "hysteria2" {
        // Mihomo v1.19.17 adapter/outbound/hysteria2.go assigns HopInterval to
        // the client only inside the nonempty Ports branch. On a single-port
        // node it has no effect; active hopping still requires a richer encoder.
        ensure!(
            uri_string(node, "ports")?.is_none_or(str::is_empty),
            "URI exporter cannot preserve node.ports"
        );
        if let Some(interval) = node.get("hop-interval").filter(|v| !v.is_null()) {
            ensure!(
                interval.as_u64().is_some(),
                "invalid URI input: hop-interval must be a nonnegative integer"
            );
        }
    }
    ensure!(
        uri_bool(node, "udp")? != Some(false),
        "URI exporter cannot preserve udp: false; use a complete configuration to retain this restriction"
    );
    let name = required(node, "name")?;
    let scheme = match kind {
        "mieru" => "mierus",
        // v2rayN SocksFmt and Sub-Store's URI producer use socks:// with
        // base64 credentials; keep this distinct from a proxy environment URL.
        "socks5" => "socks",
        "http" if uri_bool(node, "tls")? == Some(true) => "https",
        _ => kind,
    };
    let mut url = uri_endpoint(node, scheme)?;
    if kind == "ss" {
        let cipher = required(node, "cipher")?;
        let password = required(node, "password")?;
        if cipher.starts_with("2022-") {
            uri_credentials(&mut url, cipher, Some(password))?;
        } else {
            uri_credentials(
                &mut url,
                &URL_SAFE_NO_PAD.encode(format!("{cipher}:{password}")),
                None,
            )?;
        }
    } else if matches!(kind, "socks5" | "http") {
        ensure!(
            kind != "socks5" || uri_bool(node, "tls")? != Some(true),
            "URI exporter has not implemented SOCKS5 over TLS"
        );
        let username = uri_string(node, "username")?;
        let password = uri_string(node, "password")?;
        ensure!(
            password.is_none_or(str::is_empty) || username.is_some_and(|s| !s.is_empty()),
            "invalid URI input: password requires username"
        );
        if kind == "socks5" {
            let username = username.unwrap_or("");
            ensure!(
                !username.contains(':'),
                "URI exporter cannot preserve a colon in SOCKS5 username"
            );
            uri_credentials(
                &mut url,
                &URL_SAFE_NO_PAD.encode(format!("{username}:{}", password.unwrap_or(""))),
                None,
            )?;
        } else if let Some(username) = username.filter(|s| !s.is_empty()) {
            uri_credentials(&mut url, username, password)?;
        }
    } else if kind == "mieru" {
        uri_credentials(
            &mut url,
            required(node, "username")?,
            Some(required(node, "password")?),
        )?;
        let port = url.port().unwrap();
        url.set_port(None)
            .map_err(|_| anyhow::anyhow!("URI exporter cannot encode Mieru port"))?;
        let transport = required(node, "transport")?;
        ensure!(
            ["TCP", "UDP"].contains(&transport),
            "invalid URI input: transport must be TCP or UDP"
        );
        let mut q = url.query_pairs_mut();
        q.append_pair("profile", name)
            .append_pair("port", &port.to_string())
            .append_pair("protocol", transport);
        for (key, values) in [
            (
                "multiplexing",
                &[
                    "MULTIPLEXING_DEFAULT",
                    "MULTIPLEXING_OFF",
                    "MULTIPLEXING_LOW",
                    "MULTIPLEXING_MIDDLE",
                    "MULTIPLEXING_HIGH",
                ][..],
            ),
            (
                "handshake-mode",
                &[
                    "HANDSHAKE_DEFAULT",
                    "HANDSHAKE_STANDARD",
                    "HANDSHAKE_NO_WAIT",
                ][..],
            ),
        ] {
            if let Some(value) = uri_string(node, key)?.filter(|s| !s.is_empty()) {
                ensure!(values.contains(&value), "invalid URI input: {key}");
                q.append_pair(key, value);
            }
        }
    } else if kind == "hysteria2" {
        uri_credentials(&mut url, uri_string(node, "password")?.unwrap_or(""), None)?;
        let obfs = uri_string(node, "obfs")?.filter(|s| !s.is_empty());
        let obfs_password = uri_string(node, "obfs-password")?.filter(|s| !s.is_empty());
        ensure!(
            obfs.is_some() || obfs_password.is_none(),
            "invalid URI input: obfs-password requires obfs"
        );
        let mut q = url.query_pairs_mut();
        if let Some(obfs) = obfs {
            ensure!(
                ["salamander", "gecko"].contains(&obfs),
                "URI exporter has not implemented this Hysteria2 obfuscation"
            );
            let password = obfs_password
                .ok_or_else(|| anyhow::anyhow!("invalid URI input: obfs requires obfs-password"))?;
            q.append_pair("obfs", obfs)
                .append_pair("obfs-password", password);
        }
        if let Some(sni) = uri_string(node, "sni")?.filter(|s| !s.is_empty()) {
            q.append_pair("sni", sni);
        }
        if let Some(insecure) = uri_bool(node, "skip-cert-verify")? {
            q.append_pair("insecure", if insecure { "1" } else { "0" });
        }
        if let Some(pin) = uri_string(node, "fingerprint")?.filter(|s| !s.is_empty()) {
            q.append_pair("pinSHA256", pin);
        }
    } else {
        let network = uri_string(node, "network")?
            .filter(|s| !s.is_empty())
            .unwrap_or("tcp");
        ensure!(
            ["tcp", "ws"].contains(&network),
            "URI exporter has not implemented this transport"
        );
        let (path, host) = uri_ws(node, network)?;
        let tls = kind == "trojan" || uri_bool(node, "tls")?.unwrap_or(false);
        ensure!(
            kind != "trojan" || uri_bool(node, "tls")? != Some(false),
            "invalid URI input: Trojan requires TLS"
        );
        let sni = uri_sni(node)?;
        let insecure = uri_bool(node, "skip-cert-verify")?;
        let alpn = uri_alpn(node)?;
        let fingerprint = uri_string(node, "client-fingerprint")?.filter(|s| !s.is_empty());
        let reality = node.get("reality-opts").filter(|v| !v.is_null());
        ensure!(
            tls || (sni.is_none()
                && insecure != Some(true)
                && alpn.is_none()
                && fingerprint.is_none()
                && reality.is_none()),
            "URI exporter cannot preserve TLS options without tls: true"
        );
        if kind == "vmess" {
            let alter_id = match node.get("alterId") {
                None | Some(Value::Null) => 0,
                Some(value) => value.as_u64().ok_or_else(|| {
                    anyhow::anyhow!("invalid URI input: alterId must be a nonnegative integer")
                })?,
            };
            let data = serde_json::json!({"v":"2","ps":name,"add":required(node,"server")?,"port":node["port"].as_u64().unwrap().to_string(),"id":required(node,"uuid")?,"aid":alter_id.to_string(),"scy":uri_string(node,"cipher")?.unwrap_or("auto"),"net":network,"type":"none","host":host.unwrap_or_default(),"path":path,"tls":if tls {"tls"}else{""},"sni":sni.unwrap_or(""),"alpn":alpn.unwrap_or_default(),"fp":fingerprint.unwrap_or(""),"insecure":if insecure.unwrap_or(false){"1"}else{"0"}});
            return Ok(format!(
                "vmess://{}",
                STANDARD.encode(serde_json::to_vec(&data)?)
            ));
        }
        uri_credentials(
            &mut url,
            required(node, if kind == "vless" { "uuid" } else { "password" })?,
            None,
        )?;
        let mut q = url.query_pairs_mut();
        q.append_pair(
            "security",
            if reality.is_some() {
                "reality"
            } else if tls {
                "tls"
            } else {
                "none"
            },
        );
        if let Some(reality) = reality {
            uri_fields(reality, &["public-key", "short-id"], "reality-opts")?;
            let public_key = required(reality, "public-key")?;
            ensure!(
                URL_SAFE_NO_PAD
                    .decode(public_key)
                    .is_ok_and(|b| b.len() == 32),
                "invalid URI input: reality-opts.public-key"
            );
            let short_id = uri_string(reality, "short-id")?.unwrap_or("");
            ensure!(
                short_id.len() <= 16
                    && short_id.len().is_multiple_of(2)
                    && short_id.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid URI input: reality-opts.short-id"
            );
            let fp = fingerprint.ok_or_else(|| {
                anyhow::anyhow!("URI exporter requires an explicit client-fingerprint for REALITY")
            })?;
            q.append_pair("pbk", public_key)
                .append_pair("sid", short_id)
                .append_pair("fp", fp);
        } else if let Some(fp) = fingerprint {
            q.append_pair("fp", fp);
        }
        if let Some(sni) = sni {
            q.append_pair("sni", sni);
        }
        if let Some(insecure) = insecure {
            q.append_pair("allowInsecure", if insecure { "1" } else { "0" });
        }
        if let Some(alpn) = alpn {
            q.append_pair("alpn", &alpn);
        }
        if kind == "vless" {
            let encryption = uri_string(node, "encryption")?
                .filter(|s| !s.is_empty())
                .unwrap_or("none");
            ensure!(
                encryption == "none",
                "URI exporter has not implemented VLESS encryption"
            );
            q.append_pair("encryption", encryption);
            if let Some(flow) = uri_string(node, "flow")?.filter(|s| !s.is_empty()) {
                ensure!(
                    tls && network == "tcp",
                    "invalid URI input: VLESS flow requires TCP and TLS"
                );
                ensure!(
                    ["xtls-rprx-vision", "xtls-rprx-vision-udp443"].contains(&flow),
                    "URI exporter has not implemented this VLESS flow"
                );
                q.append_pair("flow", flow);
            }
        }
        q.append_pair("type", network);
        if network == "ws" {
            q.append_pair("path", &path);
            if let Some(host) = host {
                q.append_pair("host", &host);
            }
        }
    }
    let name =
        percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC).to_string();
    url.set_fragment(Some(&name));
    Ok(url.into())
}

pub fn shadowrocket_nodes(v: &Value) -> Result<String> {
    let lines = sr_nodes(v)?
        .iter()
        .map(shadowrocket_node_uri)
        .collect::<Result<Vec<_>>>()?;
    Ok(STANDARD.encode(lines.join("\n")))
}

/// Complete Shadowrocket-compatible YAML. Rule dependencies are provided by the
/// cloud resolver; this synchronous entry point handles inline-only configurations.
pub fn shadowrocket_full(v: &Value) -> Result<String> {
    let compiled = crate::config_compat::compile(
        v,
        crate::config_compat::Target::Shadowrocket,
        &Default::default(),
    )?;
    mihomo(&compiled.config, false)
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
    fn group_patch_appends_to_current_members_without_replacing_group_settings() {
        let source = "proxies: [{name: a, type: ss, server: a.example, port: 443}]\nproxy-groups: [{name: pick, type: select, proxies: [a, DIRECT], url: 'https://example.com/test', interval: 300}]\nrules: ['MATCH,pick']";
        let patch = "append-proxies: [{name: b, type: ss, server: b.example, port: 443}]\nproxy-group-patches: [{name: pick, append-proxies: [b]}]";
        let result = compose_profiles(&[source.into(), patch.into()], &BTreeMap::new()).unwrap();
        assert_eq!(
            result["proxy-groups"][0]["proxies"],
            parse("members: [a, DIRECT, b]").unwrap()["members"]
        );
        assert_eq!(result["proxy-groups"][0]["interval"], 300);
        assert_eq!(result["proxy-groups"][0]["url"], "https://example.com/test");
        assert!(result.get("proxy-group-patches").is_none());
        assert!(
            !mihomo(&result, false)
                .unwrap()
                .contains("proxy-group-patches")
        );

        // A refreshed subscription may introduce the patched member itself.
        let refreshed = source.replace("[a, DIRECT]", "[a, b, DIRECT]");
        let result = compose_profiles(&[refreshed, patch.into()], &BTreeMap::new()).unwrap();
        assert_eq!(
            result["proxy-groups"][0]["proxies"],
            parse("members: [a, b, DIRECT]").unwrap()["members"]
        );
    }
    #[test]
    fn group_patch_works_in_same_profile_and_agent_local_overlay() {
        let content = "proxy-groups: [{name: pick, type: select}]\nproxy-group-patches: [{name: pick, append-proxies: [DIRECT]}]";
        let result = compose_profiles(&[content.into()], &BTreeMap::new()).unwrap();
        assert_eq!(result["proxy-groups"][0]["proxies"][0], "DIRECT");

        let mut local = parse(&source()).unwrap();
        merge(
            &mut local,
            &parse("proxy-group-patches: [{name: pick, append-proxies: [DIRECT]}]").unwrap(),
        )
        .unwrap();
        validate(&local).unwrap();
        assert_eq!(
            local["proxy-groups"][0]["proxies"],
            parse("members: [a, DIRECT]").unwrap()["members"]
        );
        assert!(local.get("proxy-group-patches").is_none());
    }
    #[test]
    fn group_patch_rejects_invalid_shape_target_and_references() {
        let source = source();
        for bad in [
            "proxy-group-patches: wrong",
            "proxy-group-patches: [wrong]",
            "proxy-group-patches: [{name: pick}]",
            "proxy-group-patches: [{name: pick, append-proxies: wrong}]",
            "proxy-group-patches: [{name: pick, append-proxies: [a, a]}]",
            "proxy-group-patches: [{name: pick, append-proxies: [a], typo: true}]",
            "proxy-group-patches: [{name: missing, append-proxies: [a]}]",
            "proxy-group-patches: [{name: pick, append-proxies: [missing]}]",
            "proxy-group-patches: [{name: pick, append-proxies: [pick]}]",
            "proxy-group-patches: [{name: pick, append-proxies: [a]}, {name: pick, append-proxies: [DIRECT]}]",
        ] {
            assert!(
                compose_profiles(&[source.clone(), bad.into()], &BTreeMap::new()).is_err(),
                "{bad}"
            );
        }
        let invalid_group = "proxy-groups: [{name: pick, type: select, proxies: null}]\nproxy-group-patches: [{name: pick, append-proxies: [DIRECT]}]";
        assert!(compose_profiles(&[invalid_group.into()], &BTreeMap::new()).is_err());
        let mut local = parse(&source).unwrap();
        assert!(
            merge(
                &mut local,
                &parse("proxy-group-patches: [{name: unknown, append-proxies: [a]}]").unwrap()
            )
            .is_err()
        );
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

    fn export_node(fields: &str) -> Value {
        parse(&format!(
            "name: Demo\nserver: proxy.example\nport: 443\n{fields}\n"
        ))
        .unwrap()
    }

    fn uri_pairs(uri: &str) -> BTreeMap<String, String> {
        url::Url::parse(uri)
            .unwrap()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }

    #[test]
    fn shadowrocket_complete_yaml_does_not_use_the_uri_encoder() {
        let source = "proxy-providers: {}\nrule-providers: {}\nproxies:\n- {name: Socks, type: socks5, server: proxy.example, port: 1080, username: demo, password: example, udp: false}\n- {name: Mieru, type: mieru, server: proxy.example, port: 443, username: demo, password: example, transport: TCP, traffic-pattern: example}\n- {name: VMess, type: vmess, server: proxy.example, port: 443, uuid: 00000000-0000-0000-0000-000000000001, tls: true, skip-cert-verify: true}\nproxy-groups: [{name: Choice, type: select, proxies: [Socks, Mieru, VMess]}]\nrules: ['MATCH,Choice']";
        let input = compose_profiles(&[source.into()], &BTreeMap::new()).unwrap();
        let output = parse(&shadowrocket_full(&input).unwrap()).unwrap();
        assert_eq!(output["proxies"], input["proxies"]);
        assert_eq!(output["proxy-groups"], input["proxy-groups"]);
        assert_eq!(output["proxy-providers"], input["proxy-providers"]);
        assert!(output.get("rule-providers").is_none());
        assert!(shadowrocket_nodes(&input).is_err());
    }

    #[test]
    fn shadowrocket_uri_preserves_authenticated_socks_and_http_credentials() {
        for kind in ["socks5", "http"] {
            let mut node = export_node(&format!(
                "type: {kind}\nusername: 'demo@name%2F'\npassword: 'example:/?#%2F value'"
            ));
            node["name"] = "Example / + %2F 节点".into();
            node["server"] = "2001:db8::1".into();
            let uri = shadowrocket_node_uri(&node).unwrap();
            let parsed = url::Url::parse(&uri).unwrap();
            if kind == "socks5" {
                assert_eq!(parsed.scheme(), "socks");
                assert_eq!(
                    String::from_utf8(
                        URL_SAFE_NO_PAD
                            .decode(
                                percent_encoding::percent_decode_str(parsed.username())
                                    .collect::<Vec<_>>()
                            )
                            .unwrap()
                    )
                    .unwrap(),
                    "demo@name%2F:example:/?#%2F value"
                );
            } else {
                assert_eq!(parsed.scheme(), "http");
                assert_eq!(
                    percent_encoding::percent_decode_str(parsed.username())
                        .decode_utf8()
                        .unwrap(),
                    "demo@name%2F"
                );
                assert_eq!(
                    percent_encoding::percent_decode_str(parsed.password().unwrap())
                        .decode_utf8()
                        .unwrap(),
                    "example:/?#%2F value"
                );
            }
            assert_eq!(
                percent_encoding::percent_decode_str(parsed.fragment().unwrap())
                    .decode_utf8()
                    .unwrap(),
                "Example / + %2F 节点"
            );
            assert_eq!(parsed.host_str(), Some("[2001:db8::1]"));
        }
        let node = export_node("type: http\ntls: true\nusername: demo\npassword: example");
        assert!(
            shadowrocket_node_uri(&node)
                .unwrap()
                .starts_with("https://")
        );
    }

    #[test]
    fn shadowrocket_uri_vmess_insecure_sni_and_alpn_are_encoded() {
        // https://github.com/2dust/v2rayN/wiki/Description-of-VMess-share-link
        let node = export_node(
            "type: vmess\nuuid: 00000000-0000-0000-0000-000000000001\nalterId: 0\ncipher: auto\ntls: true\nsni: tls.example\nskip-cert-verify: true\nalpn: [h2, http/1.1]\nclient-fingerprint: chrome\nnetwork: ws\nws-opts: {path: '/stream?example=value', headers: {Host: front.example}}",
        );
        let uri = shadowrocket_node_uri(&node).unwrap();
        let decoded: serde_json::Value = serde_json::from_slice(
            &STANDARD
                .decode(uri.strip_prefix("vmess://").unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(decoded["insecure"], "1");
        assert_eq!(decoded["sni"], "tls.example");
        assert_eq!(decoded["alpn"], "h2,http/1.1");
        assert_eq!(decoded["fp"], "chrome");
        assert_eq!(decoded["host"], "front.example");
        assert_eq!(decoded["path"], "/stream?example=value");
        assert_eq!(decoded["aid"], "0");
    }

    #[test]
    fn shadowrocket_uri_reality_preserves_public_key_short_id_and_flow() {
        // https://github.com/XTLS/Xray-core/discussions/716
        let node = export_node(
            "type: vless\nuuid: 00000000-0000-0000-0000-000000000001\ntls: true\nservername: tls.example\nclient-fingerprint: chrome\nflow: xtls-rprx-vision\nreality-opts: {public-key: AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA, short-id: 'a1b2'}",
        );
        let uri = shadowrocket_node_uri(&node).unwrap();
        assert!(uri.starts_with("vless://00000000-0000-0000-0000-000000000001@"));
        let pairs = uri_pairs(&uri);
        assert_eq!(pairs["security"], "reality");
        assert_eq!(pairs["pbk"], "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
        assert_eq!(pairs["sid"], "a1b2");
        assert_eq!(pairs["fp"], "chrome");
        assert_eq!(pairs["flow"], "xtls-rprx-vision");
        assert_eq!(pairs["encryption"], "none");
        assert_eq!(pairs["sni"], "tls.example");
    }

    #[test]
    fn shadowrocket_uri_null_converter_placeholders_do_not_remove_valid_nodes() {
        for (fields, placeholders) in [
            (
                "type: vless\nuuid: 00000000-0000-0000-0000-000000000001\ntls: true\nservername: tls.example\nclient-fingerprint: chrome\nflow: xtls-rprx-vision\nreality-opts: {public-key: AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA, short-id: 'a1b2'}",
                "alterId: null\ncipher: null",
            ),
            (
                "type: hysteria2\npassword: example\nsni: tls.example\nobfs: salamander\nobfs-password: example-obfuscation",
                "up: null\ndown: null",
            ),
            (
                "type: socks5\nusername: demo\npassword: example\nudp: true",
                "alterId: null\ncipher: null\ndialer-proxy: null",
            ),
        ] {
            let baseline = shadowrocket_node_uri(&export_node(fields)).unwrap();
            let decorated = export_node(&format!("{fields}\n{placeholders}"));
            assert_eq!(shadowrocket_node_uri(&decorated).unwrap(), baseline);
        }
        for fields in [
            "type: vless\nuuid: null",
            "type: ss\ncipher: aes-128-gcm\npassword: null",
        ] {
            let error = shadowrocket_node_uri(&export_node(fields))
                .unwrap_err()
                .to_string();
            assert!(error.contains("missing"), "{error}");
        }
    }

    #[test]
    fn shadowrocket_uri_only_ignores_verified_vless_converter_defaults() {
        let fields = "type: vless\nuuid: 00000000-0000-0000-0000-000000000001\ntls: true\nservername: tls.example\nclient-fingerprint: chrome\nflow: xtls-rprx-vision\nreality-opts: {public-key: AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA, short-id: 'a1b2'}";
        let baseline = shadowrocket_node_uri(&export_node(fields)).unwrap();
        for cipher in ["auto", "none"] {
            let node = export_node(&format!("{fields}\nalterId: 0\ncipher: {cipher}"));
            assert_eq!(shadowrocket_node_uri(&node).unwrap(), baseline);
        }
        for invalid in [
            "alterId: 64",
            "alterId: false",
            "alterId: '0'",
            "cipher: aes-128-gcm",
            "cipher: ''",
            "cipher: false",
        ] {
            assert!(shadowrocket_node_uri(&export_node(&format!("{fields}\n{invalid}"))).is_err());
        }
        // The same fields remain connection parameters in their actual protocol.
        let vmess = export_node(
            "type: vmess\nuuid: 00000000-0000-0000-0000-000000000001\nalterId: 64\ncipher: aes-128-gcm",
        );
        let uri = shadowrocket_node_uri(&vmess).unwrap();
        let decoded: serde_json::Value = serde_json::from_slice(
            &STANDARD
                .decode(uri.strip_prefix("vmess://").unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(decoded["aid"], "64");
        assert_eq!(decoded["scy"], "aes-128-gcm");
    }

    #[test]
    fn shadowrocket_uri_hysteria2_only_ignores_inactive_port_hopping() {
        let fields = "type: hysteria2\npassword: example\nsni: tls.example\nskip-cert-verify: true\nobfs: salamander\nobfs-password: example-obfuscation";
        let baseline = shadowrocket_node_uri(&export_node(fields)).unwrap();
        for inactive in [
            "up: null\ndown: null\nhop-interval: 30",
            "ports: null\nhop-interval: 0",
            "ports: ''\nhop-interval: 15",
        ] {
            let node = export_node(&format!("{fields}\n{inactive}"));
            assert_eq!(shadowrocket_node_uri(&node).unwrap(), baseline);
        }
        for active_or_invalid in [
            "ports: '443,8443'\nhop-interval: 30",
            "ports: '4000-5000'",
            "port-hopping: true\nhop-interval: 30",
            "hop-interval: '30'",
            "hop-interval: false",
            "hop-interval: -1",
            "up: 20 Mbps",
            "down: 40 Mbps",
        ] {
            assert!(
                shadowrocket_node_uri(&export_node(&format!("{fields}\n{active_or_invalid}")))
                    .is_err()
            );
        }
    }

    #[test]
    fn shadowrocket_uri_unknown_nonnull_values_and_dialer_chains_stay_strict() {
        for value in ["false", "0", "''", "{}", "[]"] {
            let node = export_node(&format!("type: socks5\nfuture-option: {value}"));
            assert!(shadowrocket_node_uri(&node).is_err());
        }
        let node = export_node(
            "type: socks5\nusername: demo\npassword: example\nalterId: null\ncipher: null\ndialer-proxy: ExampleRelay",
        );
        let error = shadowrocket_node_uri(&node).unwrap_err().to_string();
        assert!(error.contains("node.dialer-proxy"));
        assert!(!error.contains("ExampleRelay"));
    }

    #[test]
    fn shadowrocket_uri_hysteria2_obfuscation_and_certificate_settings_round_trip() {
        // https://v2.hysteria.network/docs/developers/URI-Scheme/
        let node = export_node(
            "type: hysteria2\npassword: 'demo:example@value'\nsni: tls.example\nskip-cert-verify: true\nfingerprint: aabbcc\nobfs: salamander\nobfs-password: 'example&value'",
        );
        let uri = shadowrocket_node_uri(&node).unwrap();
        let parsed = url::Url::parse(&uri).unwrap();
        let pairs = uri_pairs(&uri);
        assert_eq!(parsed.scheme(), "hysteria2");
        assert_eq!(
            percent_encoding::percent_decode_str(parsed.username())
                .decode_utf8()
                .unwrap(),
            "demo:example@value"
        );
        assert_eq!(pairs["obfs"], "salamander");
        assert_eq!(pairs["obfs-password"], "example&value");
        assert_eq!(pairs["sni"], "tls.example");
        assert_eq!(pairs["insecure"], "1");
        assert_eq!(pairs["pinSHA256"], "aabbcc");
    }

    #[test]
    fn shadowrocket_uri_mieru_uses_official_simple_url_layout() {
        // https://github.com/enfein/mieru/blob/main/pkg/appctl/url.go
        for transport in ["TCP", "UDP"] {
            let node = export_node(&format!(
                "type: mieru\nusername: demo\npassword: 'example:@/value'\ntransport: {transport}\nmultiplexing: MULTIPLEXING_MIDDLE\nhandshake-mode: HANDSHAKE_NO_WAIT"
            ));
            let uri = shadowrocket_node_uri(&node).unwrap();
            let parsed = url::Url::parse(&uri).unwrap();
            let pairs = uri_pairs(&uri);
            assert_eq!(parsed.scheme(), "mierus");
            assert_eq!(parsed.port(), None);
            assert_eq!(parsed.username(), "demo");
            assert_eq!(
                percent_encoding::percent_decode_str(parsed.password().unwrap())
                    .decode_utf8()
                    .unwrap(),
                "example:@/value"
            );
            assert_eq!(pairs["profile"], "Demo");
            assert_eq!(pairs["port"], "443");
            assert_eq!(pairs["protocol"], transport);
            assert_eq!(pairs["multiplexing"], "MULTIPLEXING_MIDDLE");
            assert_eq!(pairs["handshake-mode"], "HANDSHAKE_NO_WAIT");
        }
    }

    #[test]
    fn shadowrocket_uri_never_discards_nested_websocket_options() {
        for kind in ["vmess", "vless", "trojan"] {
            let credential = if kind == "trojan" {
                "password: example"
            } else {
                "uuid: 00000000-0000-0000-0000-000000000001"
            };
            for ws in [
                "{headers: {Host: front.example, Authorization: 'Bearer example-private-value'}}",
                "{v2ray-http-upgrade: true}",
                "{v2ray-http-upgrade-fast-open: true}",
                "{max-early-data: 2048}",
                "{early-data-header-name: Sec-WebSocket-Protocol}",
                "{future-option: example-private-value}",
            ] {
                let node = export_node(&format!(
                    "type: {kind}\n{credential}\nnetwork: ws\nws-opts: {ws}"
                ));
                let error = shadowrocket_node_uri(&node).unwrap_err().to_string();
                assert!(error.contains("URI exporter"), "{error}");
                assert!(!error.contains("example-private-value"));
            }
        }
    }

    #[test]
    fn shadowrocket_uri_rejects_unknown_fields_and_invalid_values_without_panics_or_secrets() {
        let cases = [
            "type: socks5\nusername: demo\npassword: example\ndialer-proxy: example-private-value",
            "type: socks5\nusername: demo\npassword: example\nudp: false",
            "type: socks5\ntls: true",
            "type: socks5\nusername: [example-private-value]",
            "type: mieru\nusername: demo\npassword: example\ntransport: TCP\ntraffic-pattern: example-private-value",
            "type: hysteria2\npassword: example\nup: 20 Mbps",
            "type: vless\nuuid: example\ntls: 'true'",
            "type: vless\nuuid: example\nreality-opts: {public-key: example-private-value}",
            "type: vmess\nuuid: example\nalterId: example-private-value",
            "type: vmess\nuuid: example\nnetwork: ws\nws-opts: {headers: {Host: [example-private-value]}}",
        ];
        for fields in cases {
            let error = shadowrocket_node_uri(&export_node(fields))
                .unwrap_err()
                .to_string();
            assert!(!error.contains("example-private-value"));
        }
        for port in [
            Value::Null,
            Value::String("example-private-value".into()),
            Value::Number(0.into()),
            Value::Number(65536.into()),
        ] {
            let mut node = export_node("type: socks5");
            node["port"] = port;
            assert!(shadowrocket_node_uri(&node).is_err());
        }
    }

    #[test]
    fn shadowrocket_uri_explicit_node_list_fails_atomically() {
        let input = compose_profiles(&["proxies: [{name: Good, type: socks5, server: proxy.example, port: 1080}, {name: Unsupported, type: wireguard, server: proxy.example, port: 443}]\nrules: ['MATCH,Good']".into()], &BTreeMap::new()).unwrap();
        assert!(shadowrocket_nodes(&input).is_err());
    }

    #[test]
    fn shadowrocket_complete_diagnostics_do_not_echo_rule_contents() {
        let mut input = compose_profiles(&["proxies: [{name: Demo, type: socks5, server: proxy.example, port: 1080}]\nrules: ['GEOSITE,example-private-value,Demo', 'MATCH,Demo']".into()], &BTreeMap::new()).unwrap();
        let error = shadowrocket_full(&input).unwrap_err().to_string();
        assert!(!error.is_empty());
        assert!(!error.contains("example-private-value"));
        assert!(!error.contains("Demo"));
        input["rules"] = Value::Sequence(vec!["MATCH,Demo".into()]);
        input["example private field with secret"] = "example-private-value".into();
        let output = parse(&shadowrocket_full(&input).unwrap()).unwrap();
        assert_eq!(
            output["example private field with secret"],
            "example-private-value"
        );
    }
}
