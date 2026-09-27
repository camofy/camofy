//! Typed outbound contracts shared by sources, overlays and store packages.
//! Contracts are Camofy metadata; upstream subscriptions and client YAML stay ordinary YAML.
use crate::store::Resource;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Export {
    pub key: String,
    pub label: String,
    pub kind: String,
    pub target: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub key: String,
    pub label: String,
    pub kind: String,
    pub section: String,
    pub name: String,
    pub field: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum Binding {
    Default,
    Export { profile_id: Uuid, key: String },
    Literal { value: String },
}

fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn outbound(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && !s.contains([',', '\n', '\r'])
        && !s.chars().any(char::is_control)
}
fn label(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 120 && !s.chars().any(char::is_control)
}
pub fn validate_profile(data: &Value) -> Result<()> {
    let exports = declared_exports(data)?;
    let inputs = declared_inputs(data)?;
    ensure!(
        exports.len() <= 32 && inputs.len() <= 32,
        "too many capability declarations"
    );
    let mut keys = BTreeSet::new();
    let mut targets = BTreeSet::new();
    for e in exports {
        ensure!(
            identifier(&e.key) && label(&e.label) && outbound(&e.target),
            "invalid outbound export"
        );
        ensure!(
            ["group", "proxy"].contains(&e.kind.as_str()),
            "export kind must be group or proxy"
        );
        ensure!(keys.insert(e.key), "duplicate export key");
    }
    keys.clear();
    for i in &inputs {
        ensure!(
            identifier(&i.key) && label(&i.label) && outbound(&i.name),
            "invalid outbound input"
        );
        ensure!(i.kind == "outbound", "unsupported input kind");
        ensure!(
            ["proxies", "prepend-proxies", "append-proxies"].contains(&i.section.as_str())
                && i.field == "dialer-proxy",
            "unsupported input target"
        );
        ensure!(keys.insert(i.key.clone()), "duplicate input key");
        ensure!(
            targets.insert((i.section.clone(), i.name.clone(), i.field.clone())),
            "two inputs cannot write the same YAML field"
        );
    }
    if data["type"] == "source" {
        ensure!(
            data["inputs"].is_null(),
            "subscription sources cannot declare content inputs"
        );
    }
    if data["store"].is_object() {
        ensure!(
            data["inputs"].is_null() && data["exports"].is_null(),
            "store contracts belong to the immutable package version"
        );
    }
    if data["type"] == "overlay" && !inputs.is_empty() {
        let config =
            camofy::engine::parse(data["content"].as_str().context("overlay YAML required")?)?;
        for i in &inputs {
            let count = config[&i.section]
                .as_sequence()
                .into_iter()
                .flatten()
                .filter(|n| n["name"] == i.name)
                .count();
            ensure!(
                count == 1,
                "input {} must name exactly one proxy in {}",
                i.label,
                i.section
            );
        }
    }
    Ok(())
}
fn declared_exports(data: &Value) -> Result<Vec<Export>> {
    if data["exports"].is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(data["exports"].clone())
        .context("exports must be a list of outbound declarations")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resource(name: &str, content: &str, exports: Value, inputs: Value) -> Resource {
        Resource {
            id: Uuid::new_v4(),
            kind: "profile".into(),
            version: 1,
            data: json!({"name":name,"type":"overlay","content":content,"exports":exports,"inputs":inputs}),
        }
    }
    #[test]
    fn contracts_resolve_across_profiles_independent_of_merge_order() {
        let provider = resource(
            "provider",
            "proxy-groups: [{name: Gateway, type: select, proxies: [DIRECT]}]",
            json!([{"key":"egress","label":"Transit","kind":"group","target":"Gateway"}]),
            Value::Null,
        );
        let consumer = resource(
            "consumer",
            "prepend-proxies: [{name: Exit, type: socks5, server: example.org, port: 1080}]",
            Value::Null,
            json!([{"key":"upstream","label":"Transit","kind":"outbound","section":"prepend-proxies","name":"Exit","field":"dialer-proxy"}]),
        );
        let data = json!({"default_outbound":{"source":"export","profile_id":provider.id,"key":"egress"},"profiles":[{"profile_id":consumer.id,"enabled":true},{"profile_id":provider.id,"enabled":true}]});
        validate_profile(&consumer.data).unwrap();
        let resources = [consumer.clone(), provider.clone()];
        let resolver = Resolver::new(&resources, &data).unwrap();
        let (yaml, lock) = resolver.compile(&consumer, &data["profiles"][0]).unwrap();
        assert_eq!(
            camofy::engine::parse(&yaml).unwrap()["prepend-proxies"][0]["dialer-proxy"],
            "Gateway"
        );
        assert_eq!(lock[0]["source"]["profile_id"], provider.id.to_string());
        let mut gone = data.clone();
        gone["profiles"][1]["enabled"] = json!(false);
        assert!(
            Resolver::new(&resources, &gone)
                .unwrap()
                .compile(&consumer, &gone["profiles"][0])
                .is_err()
        );
        let changed = resource(
            "changed",
            "proxy-groups: [{name: Other, type: select, proxies: [DIRECT]}]",
            provider.data["exports"].clone(),
            Value::Null,
        );
        let records = [
            consumer,
            Resource {
                id: provider.id,
                ..changed
            },
        ];
        assert!(
            Resolver::new(&records, &data)
                .unwrap()
                .compile(&records[0], &data["profiles"][0])
                .is_err()
        );
    }
    #[test]
    fn default_is_automatic_only_when_unambiguous() {
        let a = resource(
            "a",
            "proxy-groups: [{name: Proxies, type: select, proxies: [DIRECT]}]",
            Value::Null,
            Value::Null,
        );
        let b = resource(
            "b",
            "proxy-groups: [{name: AUTO, type: select, proxies: [DIRECT]}]",
            json!([{"key":"default","label":"Main","kind":"group","target":"AUTO"}]),
            Value::Null,
        );
        let one = json!({"profiles":[{"profile_id":a.id,"enabled":true}]});
        assert_eq!(
            Resolver::new(&[a.clone()], &one)
                .unwrap()
                .resolve(None)
                .unwrap()
                .0,
            "Proxies"
        );
        let two = json!({"profiles":[{"profile_id":a.id,"enabled":true},{"profile_id":b.id,"enabled":true}]});
        assert!(Resolver::new(&[a, b], &two).unwrap().resolve(None).is_err());
    }
    #[test]
    fn export_cannot_silently_resolve_to_another_profiles_group() {
        let first = resource(
            "first",
            "proxy-groups: [{name: Gateway, type: select, proxies: [DIRECT]}]",
            json!([{"key":"egress","label":"出口","kind":"group","target":"Gateway"}]),
            Value::Null,
        );
        let second = resource(
            "second",
            "proxy-groups: [{name: Gateway, type: select, proxies: [DIRECT]}]",
            Value::Null,
            Value::Null,
        );
        let identity = json!({"profiles":[{"profile_id":first.id,"enabled":true},{"profile_id":second.id,"enabled":true}], "default_outbound":{"source":"export","profile_id":first.id,"key":"egress"}});
        assert!(dependency_lock(&[first, second], &identity).is_err());
    }
}
pub fn declared_inputs(data: &Value) -> Result<Vec<Input>> {
    if data["inputs"].is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(data["inputs"].clone())
        .context("inputs must be a list of outbound declarations")
}
fn names(data: &Value) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let content = data["content"]
        .as_str()
        .context("profile has no successfully fetched content")?;
    let config = camofy::engine::parse(content)?;
    let mut groups = BTreeSet::new();
    let mut proxies = BTreeSet::new();
    for section in [
        "proxy-groups",
        "prepend-proxy-groups",
        "append-proxy-groups",
    ] {
        for item in config[section].as_sequence().into_iter().flatten() {
            if let Some(name) = item["name"].as_str() {
                groups.insert(name.into());
            }
        }
    }
    for section in ["proxies", "prepend-proxies", "append-proxies"] {
        for item in config[section].as_sequence().into_iter().flatten() {
            if let Some(name) = item["name"].as_str() {
                proxies.insert(name.into());
            }
        }
    }
    Ok((groups, proxies))
}
fn may_export(data: &Value) -> bool {
    !data["exports"].is_null()
        || data["content"]
            .as_str()
            .is_some_and(|yaml| yaml.contains("Proxies"))
}
pub fn exports(data: &Value) -> Result<Vec<Export>> {
    let (groups, proxies) = names(data)?;
    let mut exports = declared_exports(data)?;
    // A conventional name is a useful automatic default only if its target exists.
    // Other names need an explicit user mapping; no airport-specific branch is used.
    if !exports.iter().any(|e| e.key == "default") && groups.contains("Proxies") {
        exports.push(Export {
            key: "default".into(),
            label: "默认代理出口".into(),
            kind: "group".into(),
            target: "Proxies".into(),
        });
    }
    for e in &exports {
        let found = if e.kind == "group" {
            groups.contains(&e.target)
        } else {
            proxies.contains(&e.target)
        };
        ensure!(found, "出口 {} 指向的 {} 已不存在", e.label, e.target);
    }
    Ok(exports)
}
pub fn available_exports(data: &Value) -> Value {
    if !may_export(data) {
        return json!([]);
    }
    match exports(data) {
        Ok(xs) => json!(xs),
        Err(_) => json!([]),
    }
}
pub fn validate_bindings(data: &Value) -> Result<()> {
    if !data["default_outbound"].is_null() {
        let _: Binding = serde_json::from_value(data["default_outbound"].clone())?;
    }
    for b in data["profiles"].as_array().into_iter().flatten() {
        if let Some(map) = b.get("capability_bindings") {
            let values: BTreeMap<String, Binding> = serde_json::from_value(map.clone())?;
            ensure!(
                values.len() <= 32 && values.keys().all(|k| identifier(k)),
                "invalid capability binding key"
            );
        }
    }
    Ok(())
}

pub struct Resolver<'a> {
    enabled: BTreeMap<Uuid, &'a Resource>,
    default: Option<Binding>,
}
pub fn dependency_lock(resources: &[Resource], data: &Value) -> Result<Value> {
    if !active_contracts(resources, data) {
        return Ok(json!([]));
    }
    let resolver = Resolver::new(resources, data)?;
    if data["default_outbound"].is_object() {
        resolver.resolve(None)?;
    }
    let mut lock = Vec::new();
    for b in data["profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["enabled"] == true)
    {
        let id: Uuid = b["profile_id"]
            .as_str()
            .context("profile binding needs ID")?
            .parse()?;
        let p = resolver
            .enabled
            .get(&id)
            .context("profile binding not found")?;
        if p.data["store"].is_object() {
            if let Some(bindings) = b["capability_bindings"].as_object() {
                ensure!(
                    bindings.keys().all(|key| key == "policy"),
                    "undeclared package input"
                );
            }
            if let Some(choice) = b["capability_bindings"].get("policy") {
                let (resolved, source) = resolver.resolve(Some(choice))?;
                lock.push(
                    json!({"profile_id":id,"input":"policy","resolved":resolved,"source":source}),
                );
            }
        } else if !declared_inputs(&p.data)?.is_empty() {
            lock.extend(
                resolver
                    .compile(p, b)?
                    .1
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            );
        } else {
            ensure!(
                b["capability_bindings"]
                    .as_object()
                    .is_none_or(|m| m.is_empty()),
                "profile has no declared inputs"
            );
        }
    }
    Ok(json!(lock))
}
pub fn active_contracts(resources: &[Resource], data: &Value) -> bool {
    data["default_outbound"].is_object()
        || data["profiles"].as_array().is_some_and(|bs| {
            bs.iter().any(|b| {
                b["enabled"] == true
                    && (b["capability_bindings"].is_object()
                        || resources.iter().any(|p| {
                            p.id.to_string() == b["profile_id"].as_str().unwrap_or("")
                                && p.data["inputs"].as_array().is_some_and(|xs| !xs.is_empty())
                        }))
            })
        })
}
impl<'a> Resolver<'a> {
    pub fn new(resources: &'a [Resource], data: &Value) -> Result<Self> {
        validate_bindings(data)?;
        let mut enabled = BTreeMap::new();
        for b in data["profiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["enabled"] == true)
        {
            let id: Uuid = b["profile_id"]
                .as_str()
                .context("profile binding needs ID")?
                .parse()?;
            let p = resources
                .iter()
                .find(|p| p.kind == "profile" && p.id == id)
                .context("profile binding not found")?;
            enabled.insert(id, p);
        }
        let default = if !data["default_outbound"].is_null() {
            Some(serde_json::from_value(data["default_outbound"].clone())?)
        } else {
            let mut providers = Vec::new();
            for (id, p) in &enabled {
                if p.data["store"].is_object() || !may_export(&p.data) {
                    continue;
                }
                if exports(&p.data)?.iter().any(|e| e.key == "default") {
                    providers.push(*id);
                }
            }
            if providers.len() == 1 {
                Some(Binding::Export {
                    profile_id: providers[0],
                    key: "default".into(),
                })
            } else {
                None
            }
        };
        Ok(Self { enabled, default })
    }
    pub fn resolve(&self, binding: Option<&Value>) -> Result<(String, Value)> {
        let choice: Binding = if let Some(v) = binding {
            serde_json::from_value(v.clone())?
        } else {
            Binding::Default
        };
        let choice = match choice {
            Binding::Default => self
                .default
                .as_ref()
                .context("没有唯一的默认出口；请在身份内选择出口")?,
            other => return self.resolve_explicit(&other),
        };
        self.resolve_explicit(choice)
    }
    fn resolve_explicit(&self, choice: &Binding) -> Result<(String, Value)> {
        match choice {
            Binding::Export { profile_id, key } => {
                let p = self
                    .enabled
                    .get(profile_id)
                    .context("出口提供者未在此身份中启用")?;
                let e = exports(&p.data)?
                    .into_iter()
                    .find(|e| &e.key == key)
                    .context("出口不存在或已失效")?;
                let owners = self
                    .enabled
                    .values()
                    .filter(|candidate| {
                        names(&candidate.data).is_ok_and(|(groups, proxies)| {
                            groups.contains(&e.target) || proxies.contains(&e.target)
                        })
                    })
                    .count();
                ensure!(
                    owners == 1,
                    "出口 {} 与其他 Profile 的同名节点或代理组冲突",
                    e.label
                );
                Ok((
                    e.target.clone(),
                    json!({"profile_id":profile_id,"export":key,"target":e.target,"kind":e.kind}),
                ))
            }
            Binding::Literal { value } => {
                ensure!(outbound(value), "invalid outbound name");
                Ok((value.clone(), json!({"literal":value})))
            }
            Binding::Default => bail!("default outbound cannot refer to itself"),
        }
    }
    pub fn compile(&self, profile: &Resource, binding: &Value) -> Result<(String, Value)> {
        let content = profile.data["content"]
            .as_str()
            .context("profile has no successfully fetched content")?;
        let inputs = declared_inputs(&profile.data)?;
        if inputs.is_empty() {
            ensure!(
                binding["capability_bindings"]
                    .as_object()
                    .is_none_or(|m| m.is_empty()),
                "profile has no declared inputs"
            );
            return Ok((content.to_owned(), json!([])));
        }
        let mut config = camofy::engine::parse(content)?;
        let mut lock = Vec::new();
        let allowed: BTreeSet<_> = inputs.iter().map(|i| i.key.as_str()).collect();
        if let Some(map) = binding["capability_bindings"].as_object() {
            ensure!(
                map.keys().all(|k| allowed.contains(k.as_str())),
                "undeclared capability binding"
            );
        }
        for input in inputs {
            let (value, source) = self.resolve(binding["capability_bindings"].get(&input.key))?;
            let list = config[&input.section]
                .as_sequence_mut()
                .context("input target section missing")?;
            let mut matches = list.iter_mut().filter(|n| n["name"] == input.name);
            let node = matches.next().context("input target node missing")?;
            ensure!(matches.next().is_none(), "input target name ambiguous");
            node[&input.field] = serde_yaml::Value::String(value.clone());
            lock.push(
                json!({"profile_id":profile.id,"input":input.key,"resolved":value,"source":source}),
            );
        }
        Ok((serde_yaml::to_string(&config)?, json!(lock)))
    }
}
