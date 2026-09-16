//! Versioned, bounded control messages shared by cloud and lightweight Agents.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const VERSION: u32 = 2;
pub type Selections = BTreeMap<String, String>;

pub fn validate_selections(v: &Selections) -> Result<()> {
    ensure!(v.len() <= 256, "at most 256 selections");
    for (group, node) in v {
        ensure!(
            !group.is_empty() && group.len() <= 512 && !node.is_empty() && node.len() <= 512,
            "invalid group or node name"
        );
    }
    Ok(())
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Group {
    pub name: String,
    pub kind: String,
    pub members: Vec<String>,
    pub now: Option<String>,
    #[serde(default)]
    pub dynamic: bool,
}

/// Never forward node credentials, endpoints, provider URLs or arbitrary core fields.
pub fn groups(value: &Value) -> Result<Vec<Group>> {
    let proxies = value["proxies"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("invalid core response"))?;
    let mut result = Vec::new();
    for (name, p) in proxies {
        if let Some(all) = p["all"].as_array() {
            ensure!(
                result.len() < 256 && all.len() <= 4096 && name.len() <= 512,
                "group snapshot exceeds limits"
            );
            let members = all
                .iter()
                .map(|v| {
                    let s = v
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("invalid member"))?;
                    ensure!(s.len() <= 512, "member name too long");
                    Ok(s.to_owned())
                })
                .collect::<Result<Vec<_>>>()?;
            result.push(Group {
                name: name.clone(),
                kind: p["type"].as_str().unwrap_or("Unknown").to_owned(),
                members,
                now: p["now"].as_str().map(str::to_owned),
                dynamic: false,
            });
        }
    }
    Ok(result)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcRequest {
    pub method: String,
    #[serde(default)]
    pub params: Value,
    pub idempotency_key: String,
}
impl RpcRequest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.idempotency_key.is_empty() && self.idempotency_key.len() <= 80,
            "idempotency key required"
        );
        ensure!(
            [
                "proxies.list",
                "proxies.delay",
                "core.status",
                "core.start",
                "core.stop",
                "core.restart"
            ]
            .contains(&self.method.as_str()),
            "unsupported RPC method"
        );
        if self.method == "proxies.delay" {
            let name = self.params["name"].as_str().unwrap_or("");
            ensure!(!name.is_empty() && name.len() <= 512, "node name required");
            ensure!(
                self.params.as_object().is_some_and(|p| p.len() == 1),
                "unsupported parameters"
            );
        } else {
            ensure!(
                self.params.is_null() || self.params.as_object().is_some_and(|p| p.is_empty()),
                "method takes no parameters"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub protocol: u32,
    pub binding: String,
    pub method: String,
    pub params: Value,
    pub idempotency_key: String,
    pub expected_revision: Option<String>,
    pub created_at: u64,
    pub expires_at: u64,
    pub status: String,
    #[serde(default)]
    pub result: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn snapshot_is_allowlisted() {
        let v=groups(&json!({"proxies":{"pick":{"type":"Selector","all":["a"],"now":"a","password":"never-forward"},"a":{"server":"private","password":"secret"}}})).unwrap();
        assert_eq!(v.len(), 1);
        assert!(!serde_json::to_string(&v).unwrap().contains("password"));
    }
    #[test]
    fn rpc_is_not_a_general_transport() {
        for method in ["shell", "http", "core.delete"] {
            assert!(
                RpcRequest {
                    method: method.into(),
                    params: Value::Null,
                    idempotency_key: "test".into()
                }
                .validate()
                .is_err()
            );
        }
    }
}
