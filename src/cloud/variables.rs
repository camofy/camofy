//! Identity-scoped, typed values for Profile templates. The resolver knows no
//! Mihomo field names: a Profile decides where a value is used in its YAML.
use crate::store::Resource;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    String,
    Integer,
    Number,
    Boolean,
    List,
    Object,
    Outbound,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub value_type: ValueType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum Selector {
    Literal {
        value: Value,
    },
    Pointer {
        path: String,
        expected: Option<Value>,
    },
    Named {
        section: String,
        match_field: String,
        match_value: Value,
        value_field: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Export {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub value_type: ValueType,
    pub selector: Selector,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum Binding {
    Literal { value: Value },
    Export { profile_id: Uuid, key: String },
    Identity { key: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityValue {
    #[serde(rename = "type")]
    pub value_type: ValueType,
    pub binding: Binding,
}

fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn label(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 120 && !s.chars().any(char::is_control)
}

fn matches_type(value: &Value, kind: ValueType) -> bool {
    match kind {
        ValueType::String | ValueType::Outbound => value.as_str().is_some(),
        ValueType::Integer => value.as_i64().is_some() || value.as_u64().is_some(),
        ValueType::Number => value.is_number(),
        ValueType::Boolean => value.is_boolean(),
        ValueType::List => value.is_array(),
        ValueType::Object => value.is_object(),
    }
}

fn definitions(data: &Value) -> Result<Vec<Variable>> {
    if data["store"].is_object() {
        let manifest = &data["_package"]["manifest"];
        let default = manifest["default_policy"].as_str().map(|v| json!(v));
        return Ok(vec![Variable {
            key: "policy".into(),
            label: "访问策略".into(),
            value_type: ValueType::Outbound,
            required: default.is_none(),
            default,
        }]);
    }
    if data["variables"].is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(data["variables"].clone()).context("variables must be a list")
}

fn exports(data: &Value) -> Result<Vec<Export>> {
    if data["provides"].is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(data["provides"].clone()).context("provides must be a list")
}

/// UI suggestions are syntax discovery, never a semantic export declaration.
/// No candidate becomes usable until its owner explicitly saves `provides`.
pub fn candidates(data: &Value) -> Value {
    let Some(content) = data["content"].as_str() else {
        return json!([]);
    };
    let Ok(yaml) = camofy::engine::parse(content) else {
        return json!([]);
    };
    let Ok(config) = serde_json::to_value(yaml) else {
        return json!([]);
    };
    let mut out = Vec::new();
    for (section, value) in config.as_object().into_iter().flatten() {
        if value.is_string() || value.is_number() || value.is_boolean() {
            let path = format!("/{}", section.replace('~', "~0").replace('/', "~1"));
            let kind = match value {
                Value::Bool(_) => "boolean",
                Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
                Value::Number(_) => "number",
                _ => "string",
            };
            out.push(json!({"label":section,"type":kind,
                "selector":{"source":"pointer","path":path}}));
        }
        for item in value.as_array().into_iter().flatten() {
            let Some(name) = item.get("name").and_then(Value::as_str) else {
                continue;
            };
            if !label(name) {
                continue;
            }
            let outbound = [
                "proxies",
                "prepend-proxies",
                "append-proxies",
                "proxy-groups",
                "prepend-proxy-groups",
                "append-proxy-groups",
            ]
            .contains(&section.as_str());
            out.push(json!({"label":format!("{section} / {name}"),
                "type":if outbound { "outbound" } else { "string" },
                "selector":{"source":"named","section":section,"match_field":"name",
                    "match_value":name,"value_field":"name"}}));
        }
    }
    json!(out)
}

fn references_in(s: &str) -> Result<Vec<String>> {
    let mut rest = s;
    let mut found = Vec::new();
    while let Some(start) = rest.find("{{camofy.") {
        rest = &rest[start + "{{camofy.".len()..];
        let end = rest.find("}}").context("unclosed Camofy variable")?;
        let key = &rest[..end];
        ensure!(identifier(key), "invalid Camofy variable name");
        found.push(key.to_string());
        rest = &rest[end + 2..];
    }
    Ok(found)
}

fn check_template(value: &serde_yaml::Value, declared: &BTreeSet<&str>) -> Result<()> {
    match value {
        serde_yaml::Value::String(s) => {
            for key in references_in(s)? {
                ensure!(declared.contains(key.as_str()), "undeclared variable {key}");
            }
        }
        serde_yaml::Value::Sequence(xs) => {
            for item in xs {
                check_template(item, declared)?;
            }
        }
        serde_yaml::Value::Mapping(xs) => {
            for (key, value) in xs {
                // Dynamic keys would make merge semantics and provenance ambiguous.
                if let serde_yaml::Value::String(key) = key {
                    ensure!(!key.contains("{{camofy."), "variables cannot be YAML keys");
                }
                check_template(value, declared)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn validate_profile(data: &Value) -> Result<()> {
    let variables = definitions(data)?;
    let provided = exports(data)?;
    ensure!(
        variables.len() <= 64 && provided.len() <= 64,
        "too many variables or exports"
    );
    let mut keys = BTreeSet::new();
    for item in &variables {
        ensure!(
            identifier(&item.key) && label(&item.label),
            "invalid variable declaration"
        );
        ensure!(keys.insert(&item.key), "duplicate variable key");
        if let Some(default) = &item.default {
            ensure!(
                matches_type(default, item.value_type),
                "variable default type mismatch"
            );
        }
    }
    keys.clear();
    for item in &provided {
        ensure!(
            identifier(&item.key) && label(&item.label),
            "invalid export declaration"
        );
        ensure!(keys.insert(&item.key), "duplicate export key");
        match &item.selector {
            Selector::Literal { value } => {
                ensure!(
                    matches_type(value, item.value_type),
                    "export value type mismatch"
                );
            }
            Selector::Pointer { path, .. } => {
                ensure!(
                    path.starts_with('/') && path.len() <= 512,
                    "invalid export pointer"
                );
            }
            Selector::Named {
                section,
                match_field,
                value_field,
                ..
            } => {
                ensure!(
                    label(section) && label(match_field) && label(value_field),
                    "invalid export selector"
                );
            }
        }
    }
    if data["type"] == "source" {
        ensure!(
            variables.is_empty(),
            "upstream subscription content cannot contain Camofy variables"
        );
        if let Some(content) = data["content"].as_str() {
            let config = serde_json::to_value(camofy::engine::parse(content)?)?;
            for item in &provided {
                let value = select(&config, &item.selector)?;
                ensure!(
                    matches_type(&value, item.value_type),
                    "export {} type mismatch",
                    item.label
                );
            }
        }
    }
    if data["store"].is_object() {
        ensure!(
            data["variables"].is_null() && data["provides"].is_null(),
            "Store contracts belong to an immutable package version"
        );
    }
    if data["type"] == "overlay" && !data["store"].is_object() {
        let content = data["content"].as_str().context("overlay YAML required")?;
        let yaml = camofy::engine::parse(content)?;
        let declared = variables.iter().map(|v| v.key.as_str()).collect();
        check_template(&yaml, &declared)?;
    }
    Ok(())
}

pub fn validate_identity(data: &Value) -> Result<()> {
    let aliases = data["identity_values"].as_object();
    ensure!(
        aliases.is_none_or(|m| m.len() <= 64),
        "too many identity values"
    );
    for (key, value) in aliases.into_iter().flatten() {
        ensure!(identifier(key), "invalid identity value name");
        let _: IdentityValue = serde_json::from_value(value.clone())?;
    }
    for item in data["profiles"].as_array().into_iter().flatten() {
        if let Some(bindings) = item.get("variable_bindings") {
            let map: BTreeMap<String, Binding> = serde_json::from_value(bindings.clone())?;
            ensure!(
                map.len() <= 64 && map.keys().all(|key| identifier(key)),
                "invalid variable bindings"
            );
        }
    }
    Ok(())
}

fn select(config: &Value, selector: &Selector) -> Result<Value> {
    match selector {
        Selector::Literal { value } => Ok(value.clone()),
        Selector::Pointer { path, expected } => {
            let value = config
                .pointer(path)
                .context("export pointer no longer exists")?;
            if let Some(expected) = expected {
                ensure!(
                    value == expected,
                    "export pointer now identifies a different value"
                );
            }
            Ok(value.clone())
        }
        Selector::Named {
            section,
            match_field,
            match_value,
            value_field,
        } => {
            let items = config
                .get(section)
                .and_then(Value::as_array)
                .context("export collection no longer exists")?;
            let mut matches = items
                .iter()
                .filter(|item| item.get(match_field) == Some(match_value));
            let item = matches.next().context("export target no longer exists")?;
            ensure!(matches.next().is_none(), "export target is ambiguous");
            Ok(item
                .get(value_field)
                .context("export value no longer exists")?
                .clone())
        }
    }
}

fn replacement(value: &Value) -> Result<String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => bail!("list and object variables require a whole YAML value"),
    }
}

fn substitute(value: &mut serde_yaml::Value, values: &BTreeMap<String, Value>) -> Result<()> {
    match value {
        serde_yaml::Value::String(s) => {
            let keys = references_in(s)?;
            if keys.is_empty() {
                return Ok(());
            }
            if keys.len() == 1 && *s == format!("{{{{camofy.{}}}}}", keys[0]) {
                *value =
                    serde_yaml::to_value(values.get(&keys[0]).context("variable not resolved")?)?;
            } else {
                let mut result = s.clone();
                for key in keys {
                    let resolved = replacement(values.get(&key).context("variable not resolved")?)?;
                    result = result.replace(&format!("{{{{camofy.{key}}}}}"), &resolved);
                }
                *s = result;
            }
        }
        serde_yaml::Value::Sequence(xs) => {
            for item in xs {
                substitute(item, values)?;
            }
        }
        serde_yaml::Value::Mapping(xs) => {
            for item in xs.values_mut() {
                substitute(item, values)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub struct Resolver<'a> {
    identity: &'a Value,
    enabled: BTreeMap<Uuid, &'a Resource>,
}

impl<'a> Resolver<'a> {
    pub fn new(resources: &'a [Resource], identity: &'a Value) -> Result<Self> {
        validate_identity(identity)?;
        let mut enabled = BTreeMap::new();
        for item in identity["profiles"].as_array().into_iter().flatten() {
            if item["enabled"] != true {
                continue;
            }
            let id: Uuid = item["profile_id"]
                .as_str()
                .context("profile ID required")?
                .parse()?;
            let profile = resources
                .iter()
                .find(|r| r.id == id && r.kind == "profile")
                .context("profile not found")?;
            enabled.insert(id, profile);
        }
        Ok(Self { identity, enabled })
    }

    fn binding(&self, id: Uuid) -> Result<&Value> {
        self.identity["profiles"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|item| item["enabled"] == true && item["profile_id"] == id.to_string())
            .context("profile not enabled in identity")
    }

    fn resolve_binding(&self, choice: &Binding, stack: &mut Vec<String>) -> Result<Value> {
        match choice {
            Binding::Literal { value } => Ok(value.clone()),
            Binding::Identity { key } => {
                ensure!(identifier(key), "invalid identity value reference");
                let marker = format!("identity:{key}");
                ensure!(!stack.contains(&marker), "identity value cycle");
                let item: IdentityValue =
                    serde_json::from_value(self.identity["identity_values"][key].clone())
                        .context("identity value not found")?;
                stack.push(marker);
                let result = self.resolve_binding(&item.binding, stack);
                stack.pop();
                let value = result?;
                ensure!(
                    matches_type(&value, item.value_type),
                    "identity value type mismatch"
                );
                Ok(value)
            }
            Binding::Export { profile_id, key } => {
                let marker = format!("export:{profile_id}:{key}");
                ensure!(
                    !stack.contains(&marker),
                    "Profile variable dependency cycle"
                );
                let profile = self
                    .enabled
                    .get(profile_id)
                    .context("export provider is not enabled in this identity")?;
                let item = exports(&profile.data)?
                    .into_iter()
                    .find(|item| item.key == *key)
                    .context("export not declared by provider")?;
                stack.push(marker);
                let result = (|| {
                    let content = self.render(profile, stack)?;
                    let yaml = camofy::engine::parse(&content)?;
                    let value = select(&serde_json::to_value(yaml)?, &item.selector)?;
                    ensure!(
                        matches_type(&value, item.value_type),
                        "export type mismatch"
                    );
                    Ok(value)
                })();
                stack.pop();
                result
            }
        }
    }

    fn render(&self, profile: &Resource, stack: &mut Vec<String>) -> Result<String> {
        if profile.data["type"] == "source" {
            return Ok(profile.data["content"]
                .as_str()
                .context("profile has no YAML content")?
                .to_string());
        }
        let values = self.values(profile, stack)?;
        if profile.data["store"].is_object() {
            return crate::catalog::compile(profile, values.get("policy").and_then(Value::as_str));
        }
        let content = profile.data["content"]
            .as_str()
            .context("profile has no YAML content")?;
        let mut yaml = camofy::engine::parse(&content)?;
        substitute(&mut yaml, &values)?;
        Ok(serde_yaml::to_string(&yaml)?)
    }

    fn values(
        &self,
        profile: &Resource,
        stack: &mut Vec<String>,
    ) -> Result<BTreeMap<String, Value>> {
        let variables = definitions(&profile.data)?;
        let mut values = BTreeMap::new();
        let binding = self.binding(profile.id)?;
        let bindings = binding["variable_bindings"].as_object();
        let allowed: BTreeSet<_> = variables.iter().map(|v| v.key.as_str()).collect();
        ensure!(
            bindings.is_none_or(|m| m.keys().all(|key| allowed.contains(key.as_str()))),
            "binding names an undeclared variable"
        );
        for item in variables {
            let value = if let Some(choice) = bindings.and_then(|m| m.get(&item.key)) {
                let choice: Binding = serde_json::from_value(choice.clone())?;
                self.resolve_binding(&choice, stack)?
            } else if let Some(default) = &item.default {
                default.clone()
            } else if item.required {
                bail!(
                    "variable {} requires a binding in this identity",
                    item.label
                );
            } else {
                continue;
            };
            ensure!(
                matches_type(&value, item.value_type),
                "variable {} type mismatch",
                item.label
            );
            values.insert(item.key, value);
        }
        Ok(values)
    }

    pub fn compile(&self, profile: &Resource) -> Result<String> {
        self.render(profile, &mut Vec::new())
    }

    pub fn lock(&self) -> Result<Value> {
        let mut items = Vec::new();
        for key in self.identity["identity_values"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, _)| key)
        {
            self.resolve_binding(&Binding::Identity { key: key.clone() }, &mut Vec::new())?;
        }
        for (id, profile) in &self.enabled {
            if profile.data["variables"].is_null()
                && self.binding(*id)?["variable_bindings"].is_null()
            {
                continue;
            }
            self.compile(profile)?;
            items.push(json!({"profile_id":id,"version":profile.version,
                "inputs":definitions(&profile.data)?.iter().map(|v| &v.key).collect::<Vec<_>>() }));
        }
        Ok(json!(items))
    }

    pub fn explain(&self) -> Result<Value> {
        let mut out = Vec::new();
        for (id, profile) in &self.enabled {
            let values = self.values(profile, &mut Vec::new())?;
            let binding = self.binding(*id)?;
            for (key, value) in values {
                let source = if !binding["variable_bindings"][&key].is_null() {
                    binding["variable_bindings"][&key].clone()
                } else {
                    json!({"source":"declared_default"})
                };
                out.push(json!({"profile_id":id,"input":key,"value":value,"binding":source}));
            }
        }
        Ok(json!(out))
    }

    pub fn validate_outbounds(&self, rendered: &[(Uuid, String)]) -> Result<()> {
        let mut owners: BTreeMap<String, BTreeSet<Uuid>> = BTreeMap::new();
        for (id, content) in rendered {
            let yaml = camofy::engine::parse(content)?;
            for section in [
                "proxies",
                "prepend-proxies",
                "append-proxies",
                "proxy-groups",
                "prepend-proxy-groups",
                "append-proxy-groups",
            ] {
                for item in yaml[section].as_sequence().into_iter().flatten() {
                    if let Some(name) = item["name"].as_str() {
                        owners.entry(name.to_string()).or_default().insert(*id);
                    }
                }
            }
        }
        let builtins = [
            "DIRECT",
            "REJECT",
            "REJECT-DROP",
            "PASS",
            "COMPATIBLE",
            "GLOBAL",
        ];
        for (id, profile) in &self.enabled {
            let values = self.values(profile, &mut Vec::new())?;
            for declaration in definitions(&profile.data)? {
                if declaration.value_type != ValueType::Outbound {
                    continue;
                }
                let Some(value) = values.get(&declaration.key).and_then(Value::as_str) else {
                    continue;
                };
                if builtins.contains(&value) {
                    continue;
                }
                let count = owners.get(value).map_or(0, BTreeSet::len);
                ensure!(
                    count == 1,
                    "outbound variable {} must resolve to one unambiguous node or group",
                    declaration.label
                );
                let binding = self.binding(*id)?;
                if let Some(raw) = binding["variable_bindings"].get(&declaration.key) {
                    let choice: Binding = serde_json::from_value(raw.clone())?;
                    if let Some(provider_id) = self.provider_of(&choice, &mut Vec::new())? {
                        ensure!(
                            owners[value].contains(&provider_id),
                            "outbound variable {} no longer points to its selected Profile",
                            declaration.label
                        );
                    }
                }
            }
        }
        for (key, item) in self.identity["identity_values"]
            .as_object()
            .into_iter()
            .flatten()
        {
            let definition: IdentityValue = serde_json::from_value(item.clone())?;
            if definition.value_type != ValueType::Outbound {
                continue;
            }
            let value =
                self.resolve_binding(&Binding::Identity { key: key.clone() }, &mut Vec::new())?;
            let name = value
                .as_str()
                .context("outbound identity value must be text")?;
            if builtins.contains(&name) {
                continue;
            }
            ensure!(
                owners.get(name).map_or(0, BTreeSet::len) == 1,
                "identity outbound {key} must name one unambiguous node or group"
            );
            if let Some(provider) = self.provider_of(&definition.binding, &mut Vec::new())? {
                ensure!(
                    owners[name].contains(&provider),
                    "identity outbound {key} no longer points to its selected Profile"
                );
            }
        }
        Ok(())
    }

    fn provider_of(&self, choice: &Binding, stack: &mut Vec<String>) -> Result<Option<Uuid>> {
        match choice {
            Binding::Export { profile_id, .. } => Ok(Some(*profile_id)),
            Binding::Identity { key } => {
                ensure!(!stack.contains(key), "identity value cycle");
                let item: IdentityValue =
                    serde_json::from_value(self.identity["identity_values"][key].clone())?;
                stack.push(key.clone());
                let result = self.provider_of(&item.binding, stack);
                stack.pop();
                result
            }
            Binding::Literal { .. } => Ok(None),
        }
    }
}

pub fn active(resources: &[Resource], identity: &Value) -> bool {
    identity["identity_values"].is_object()
        || identity["profiles"].as_array().is_some_and(|items| {
            items.iter().any(|item| {
                item["enabled"] == true
                    && (item["variable_bindings"].is_object()
                        || resources.iter().any(|profile| {
                            profile.id.to_string() == item["profile_id"]
                                && (profile.data["variables"].is_array()
                                    || profile.data["store"].is_object())
                        }))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str, data: Value) -> Resource {
        Resource {
            id: Uuid::new_v4(),
            kind: "profile".into(),
            version: 1,
            data: json!({"name":name,"type":"overlay","content":"{}"})
                .as_object()
                .unwrap()
                .iter()
                .fold(data, |mut data, (k, v)| {
                    if data.get(k).is_none() {
                        data[k] = v.clone();
                    }
                    data
                }),
        }
    }

    #[test]
    fn typed_values_are_placed_where_the_template_asks() {
        let provider = profile(
            "source",
            json!({"type":"source",
            "content":"proxy-groups: [{name: Gateway, type: select, proxies: [DIRECT]}]",
            "provides":[{"key":"egress","label":"Egress","type":"outbound",
                "selector":{"source":"named","section":"proxy-groups","match_field":"name",
                    "match_value":"Gateway","value_field":"name"}}]}),
        );
        let consumer = profile(
            "custom",
            json!({"type":"overlay",
            "content":"mixed-port: '{{camofy.port}}'\nallow-lan: '{{camofy.lan}}'\nproxy-groups: [{name: Choice, type: select, proxies: ['{{camofy.route}}', DIRECT]}]\nrules: ['DOMAIN,example.org,{{camofy.route}}']",
            "variables":[{"key":"port","label":"Port","type":"integer","required":true},
                {"key":"lan","label":"LAN","type":"boolean","required":true},
                {"key":"route","label":"Route","type":"outbound","required":true}]}),
        );
        validate_profile(&consumer.data).unwrap();
        validate_profile(&provider.data).unwrap();
        let identity = json!({"profiles":[
            {"profile_id":consumer.id,"enabled":true,"variable_bindings":{
                "port":{"source":"literal","value":7890},
                "lan":{"source":"literal","value":false},
                "route":{"source":"identity","key":"main"}}},
            {"profile_id":provider.id,"enabled":true}],
            "identity_values":{"main":{"type":"outbound","binding":{
                "source":"export","profile_id":provider.id,"key":"egress"}}}});
        let resources = [consumer.clone(), provider];
        let resolver = Resolver::new(&resources, &identity).unwrap();
        let rendered = camofy::engine::parse(&resolver.compile(&consumer).unwrap()).unwrap();
        assert_eq!(rendered["mixed-port"].as_i64(), Some(7890));
        assert_eq!(rendered["allow-lan"].as_bool(), Some(false));
        assert_eq!(
            rendered["proxy-groups"][0]["proxies"][0].as_str(),
            Some("Gateway")
        );
        assert_eq!(
            rendered["rules"][0].as_str(),
            Some("DOMAIN,example.org,Gateway")
        );
        assert!(!resolver.lock().unwrap().to_string().contains("Gateway"));
    }

    #[test]
    fn no_name_is_automatically_promoted_to_an_export() {
        let source = json!({"type":"source",
            "content":"proxy-groups: [{name: Proxies, type: select, proxies: [DIRECT]}]"});
        assert!(exports(&source).unwrap().is_empty());
        assert!(
            candidates(&source)
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["label"] == "proxy-groups / Proxies")
        );
        let consumer = profile(
            "custom",
            json!({"content":"mixed-port: '{{camofy.port}}'",
            "variables":[{"key":"port","label":"Port","type":"integer","required":true}]}),
        );
        let identity = json!({"profiles":[{"profile_id":consumer.id,"enabled":true}]});
        let resources = [consumer.clone()];
        assert!(
            Resolver::new(&resources, &identity)
                .unwrap()
                .compile(&consumer)
                .is_err()
        );
    }

    #[test]
    fn reference_cycles_and_type_mismatch_are_rejected() {
        let a = profile(
            "a",
            json!({"content":"mixed-port: '{{camofy.port}}'",
            "variables":[{"key":"port","label":"Port","type":"integer","required":true}]}),
        );
        let resources = [a.clone()];
        let bad_type = json!({"profiles":[{"profile_id":a.id,"enabled":true,
            "variable_bindings":{"port":{"source":"literal","value":"7890"}}}]});
        assert!(
            Resolver::new(&resources, &bad_type)
                .unwrap()
                .compile(&a)
                .is_err()
        );
        let cycle = json!({"profiles":[{"profile_id":a.id,"enabled":true,
            "variable_bindings":{"port":{"source":"identity","key":"first"}}}],
            "identity_values":{"first":{"type":"integer","binding":{
                "source":"identity","key":"second"}},
                "second":{"type":"integer","binding":{"source":"identity","key":"first"}}}});
        assert!(
            Resolver::new(&resources, &cycle)
                .unwrap()
                .compile(&a)
                .is_err()
        );
    }

    #[test]
    fn untrusted_source_content_is_not_interpolated() {
        let source = profile(
            "source",
            json!({"type":"source",
            "content":"rule-providers: {x: {url: '{{camofy.secret}}'}}"}),
        );
        let identity = json!({"profiles":[{"profile_id":source.id,"enabled":true}]});
        let resources = [source.clone()];
        assert!(
            Resolver::new(&resources, &identity)
                .unwrap()
                .compile(&source)
                .unwrap()
                .contains("{{camofy.secret}}")
        );
    }

    #[test]
    fn outbound_references_preserve_provider_identity_across_merge_order() {
        let source = profile(
            "airport",
            json!({"type":"source",
            "content":"proxy-groups: [{name: Gateway, type: select, proxies: [DIRECT]}]",
            "provides":[{"key":"main","label":"Chosen group","type":"outbound",
                "selector":{"source":"named","section":"proxy-groups","match_field":"name",
                    "match_value":"Gateway","value_field":"name"}}]}),
        );
        let consumer = profile(
            "relay",
            json!({"content":"prepend-proxies: [{name: Relay, type: socks5, server: example.org, port: 1080, dialer-proxy: '{{camofy.upstream}}'}]",
            "variables":[{"key":"upstream","label":"Transit","type":"outbound","required":true}]}),
        );
        let identity = json!({"profiles":[
            {"profile_id":consumer.id,"enabled":true,"variable_bindings":{"upstream":{"source":"export","profile_id":source.id,"key":"main"}}},
            {"profile_id":source.id,"enabled":true}]});
        let resources = [consumer.clone(), source.clone()];
        let resolver = Resolver::new(&resources, &identity).unwrap();
        let rendered = vec![
            (consumer.id, resolver.compile(&consumer).unwrap()),
            (source.id, resolver.compile(&source).unwrap()),
        ];
        resolver.validate_outbounds(&rendered).unwrap();
        let mut colliding = rendered;
        colliding.push((
            Uuid::new_v4(),
            "proxy-groups: [{name: Gateway, type: select, proxies: [DIRECT]}]".into(),
        ));
        assert!(resolver.validate_outbounds(&colliding).is_err());
        let mut wrong_owner = colliding;
        wrong_owner.remove(1);
        assert!(resolver.validate_outbounds(&wrong_owner).is_err());
    }

    #[test]
    fn malformed_contracts_and_missing_targets_fail_before_publish() {
        let template = profile(
            "template",
            json!({"content":"mixed-port: '{{camofy.listen}}'",
            "variables":[{"key":"listen","label":"Listen port","type":"integer","required":true}]}),
        );
        assert!(validate_profile(&template.data).is_ok());
        let mut undeclared = template.data.clone();
        undeclared["content"] = json!("mixed-port: '{{camofy.other}}'");
        assert!(validate_profile(&undeclared).is_err());
        let mut dynamic_key = template.data.clone();
        dynamic_key["content"] = json!("'{{camofy.listen}}': value");
        assert!(validate_profile(&dynamic_key).is_err());
        let source = profile(
            "source",
            json!({"type":"source",
            "content":"proxy-groups: [{name: New, type: select, proxies: [DIRECT]}]",
            "provides":[{"key":"route","label":"Route","type":"outbound",
                "selector":{"source":"named","section":"proxy-groups","match_field":"name",
                    "match_value":"Old","value_field":"name"}}]}),
        );
        assert!(validate_profile(&source.data).is_err());
    }

    #[test]
    fn rule_package_policy_uses_the_same_identity_binding_contract() {
        let provider = profile(
            "source",
            json!({"type":"source",
            "content":"proxy-groups: [{name: Route, type: select, proxies: [DIRECT]}]",
            "provides":[{"key":"route","label":"Route","type":"outbound",
                "selector":{"source":"named","section":"proxy-groups","match_field":"name",
                    "match_value":"Route","value_field":"name"}}]}),
        );
        let package = profile(
            "rule package",
            json!({"store":{"slug":"sample","version_id":Uuid::new_v4().to_string()},
            "_package":{"manifest":{"name":"Sample","summary":"A rule","category":"test","notes":"none","default_policy":null,"sources":[]},
            "rules":[{"kind":"DOMAIN","value":"example.org","no_resolve":false}]}}),
        );
        let identity = json!({"profiles":[{"profile_id":package.id,"enabled":true,
            "variable_bindings":{"policy":{"source":"export","profile_id":provider.id,"key":"route"}}},
            {"profile_id":provider.id,"enabled":true}]});
        let resources = [package.clone(), provider];
        let resolver = Resolver::new(&resources, &identity).unwrap();
        let content = resolver.compile(&package).unwrap();
        assert!(content.contains("DOMAIN,example.org,Route"));
    }
}
