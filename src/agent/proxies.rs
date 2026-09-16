use super::{Agent, atomic, now};
use anyhow::{Result, ensure};
use camofy::protocol::{Group, Job, Selections};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, time::Duration};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Durable {
    pub binding: String,
    pub identity: String,
    pub selection_version: u64,
    pub override_version: u64,
    pub selections: Selections,
    pub overrides: Selections,
    pub pending: BTreeMap<String, Option<String>>,
    pub receipts: BTreeMap<String, Value>,
    #[serde(skip)]
    report_hash: String,
    #[serde(skip)]
    report_at: u64,
}
impl Durable {
    pub async fn load(path: &Path) -> Result<Self> {
        let mut state: Self = match tokio::fs::read(path.join("proxies.json")).await {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e.into()),
        };
        for result in state.receipts.values_mut() {
            if result["status"] == "executing" {
                *result = json!({"status":"unknown","error":"Agent restarted before completion could be confirmed"});
            }
        }
        Ok(state)
    }
    fn effective_overrides(&self) -> Selections {
        let mut result = self.overrides.clone();
        for (group, node) in &self.pending {
            if let Some(node) = node {
                result.insert(group.clone(), node.clone());
            } else {
                result.remove(group);
            }
        }
        result
    }
    pub(super) fn effective(&self) -> Selections {
        self.selections.clone()
    }
}
impl Agent {
    async fn save_proxies(&self) -> Result<()> {
        atomic(
            &self.s.data_dir.join("proxies.json"),
            &serde_json::to_vec(&self.proxies)?,
        )
        .await
    }
    pub(super) async fn accept_control(&mut self, c: &Value) -> Result<()> {
        if c["protocol"] != 2 {
            return Ok(());
        }
        let binding = c["binding"].as_str().unwrap_or("");
        if self.proxies.binding != binding {
            self.proxies = Durable {
                binding: binding.into(),
                identity: c["identity"].as_str().unwrap_or("").into(),
                ..Default::default()
            };
        }
        self.proxies.selection_version = c["selection_version"].as_u64().unwrap_or(0);
        self.proxies.override_version = c["override_version"].as_u64().unwrap_or(0);
        self.proxies.selections = serde_json::from_value(
            c["selections"]
                .as_object()
                .map(|o| json!(o))
                .unwrap_or(json!({})),
        )?;
        self.proxies.overrides = serde_json::from_value(
            c["overrides"]
                .as_object()
                .map(|o| json!(o))
                .unwrap_or(json!({})),
        )?;
        // Migrate old local/remote overrides without replaying them after reconnect.
        self.proxies.overrides.clear();
        self.proxies.pending.clear();
        self.save_proxies().await?;
        Ok(())
    }
    async fn core_json(&self, path: &str) -> Result<Value> {
        let mut response = self
            .http
            .get(self.core(path))
            .bearer_auth(&self.secret)
            .timeout(Duration::from_secs(3))
            .send()
            .await?
            .error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 2 * 1024 * 1024,
                "core response too large"
            );
            bytes.extend(chunk);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
    async fn proxy_groups(&self) -> Result<Vec<Group>> {
        camofy::protocol::groups(&self.core_json("/proxies").await?)
    }
    async fn select_one(&self, group: &str, node: &str) -> Result<()> {
        let mut u = url::Url::parse(&self.core("/proxies/"))?;
        u.path_segments_mut().unwrap().pop_if_empty().push(group);
        self.http
            .put(u)
            .bearer_auth(&self.secret)
            .json(&json!({"name":node}))
            .timeout(Duration::from_secs(3))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
    pub(super) async fn reconcile_proxies(&mut self) {
        let mut errors = BTreeMap::<String, String>::new();
        let mut desired = self.proxies.effective();
        // Clearing a choice means configuration default, not a stale device cache.
        if let Some(cache) = &self.cached
            && let Ok(config) = camofy::engine::parse(&cache.content)
        {
            for group in config["proxy-groups"].as_sequence().into_iter().flatten() {
                if group["type"] == "select"
                    && let (Some(name), Some(first)) = (
                        group["name"].as_str(),
                        group["default-selected"].as_str().or_else(|| {
                            group["proxies"]
                                .as_sequence()
                                .and_then(|v| v.first())
                                .and_then(|v| v.as_str())
                        }),
                    )
                {
                    desired.entry(name.into()).or_insert_with(|| first.into());
                }
            }
        }
        let mut groups = Vec::new();
        let status = if self.control.stopped && self.child.is_some() {
            "stopping"
        } else if self.control.stopped {
            "stopped"
        } else {
            match self.proxy_groups().await {
                Err(_) => "unavailable",
                Ok(initial) => {
                    let mut changed = false;
                    let mut attempts = 0;
                    for (name, node) in &desired {
                        match initial.iter().find(|g| g.name == *name) {
                            None => {
                                errors.insert(name.clone(), "group_missing".into());
                            }
                            Some(g) if g.kind != "Selector" => {
                                errors.insert(name.clone(), "not_selectable".into());
                            }
                            Some(g) if !g.members.contains(node) => {
                                errors.insert(name.clone(), "node_missing".into());
                            }
                            Some(g) if g.now.as_ref() != Some(node) => {
                                if attempts >= 8 {
                                    errors.insert(name.clone(), "pending".into());
                                    continue;
                                }
                                attempts += 1;
                                if self.select_one(name, node).await.is_err() {
                                    errors.insert(name.clone(), "apply_failed".into());
                                }
                                changed = true;
                            }
                            _ => {}
                        }
                    }
                    groups = if changed {
                        self.proxy_groups().await.unwrap_or_default()
                    } else {
                        initial
                    };
                    for (name, node) in &desired {
                        if !errors.contains_key(name)
                            && !groups
                                .iter()
                                .any(|g| g.name == *name && g.now.as_ref() == Some(node))
                        {
                            errors.insert(name.clone(), "readback_mismatch".into());
                        }
                    }
                    if errors.is_empty() {
                        "applied"
                    } else {
                        "partial"
                    }
                }
            }
        };
        let state = json!({"groups":groups,"status":status,"errors":errors,"selection_version":self.proxies.selection_version,"override_version":self.proxies.override_version,"overrides":self.proxies.effective_overrides(),"pending_local":!self.proxies.pending.is_empty(),"desired":desired,"binding":self.proxies.binding,"revision":self.cached.as_ref().map(|c|&c.revision),"sampled_at":now()});
        self.local.status.lock().await["proxy_state"] = state.clone();
        // No logs, node credentials or connection metadata leave the device.
        let mut fingerprint = state.clone();
        fingerprint.as_object_mut().unwrap().remove("sampled_at");
        let hash = camofy::digest(serde_json::to_vec(&fingerprint).unwrap_or_default());
        if !self.proxies.binding.is_empty()
            && (self.proxies.report_hash != hash
                || now().saturating_sub(self.proxies.report_at) >= 300)
            && self.send_control(json!({"state":state})).await.is_ok()
        {
            self.proxies.report_hash = hash;
            self.proxies.report_at = now();
        }
    }
    async fn send_control(&self, mut body: Value) -> Result<()> {
        body["binding"] = json!(self.proxies.binding);
        self.http
            .post(self.api("/api/sync/control"))
            .bearer_auth(&self.s.token)
            .timeout(Duration::from_secs(3))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
    pub(super) async fn local_proxy(&mut self, method: &str, params: Value) -> Result<Value> {
        match method {
            "proxies.delay" => {
                let name = params["name"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("node required"))?;
                ensure!(name.len() <= 512, "name too long");
                let mut u = url::Url::parse(&self.core("/proxies/"))?;
                u.path_segments_mut()
                    .unwrap()
                    .pop_if_empty()
                    .push(name)
                    .push("delay");
                let value: Value = self
                    .http
                    .get(u)
                    .bearer_auth(&self.secret)
                    .query(&[
                        ("url", "https://www.gstatic.com/generate_204"),
                        ("timeout", "3000"),
                    ])
                    .timeout(Duration::from_secs(4))
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                Ok(json!({"name":name,"delay":value["delay"],"sampled_at":now()}))
            }
            "proxies.list" => {
                self.reconcile_proxies().await;
                Ok(self.local.status.lock().await["proxy_state"].clone())
            }
            "proxies.select" => {
                anyhow::bail!("节点选择由云端身份统一管理，请前往身份的代理分组页面");
            }
            _ => anyhow::bail!("unsupported local operation"),
        }
    }
    pub(super) async fn process_jobs(&mut self, value: &Value) -> Result<()> {
        let mut jobs: Vec<Job> =
            serde_json::from_value(value.as_array().map(|a| json!(a)).unwrap_or(json!([])))?;
        jobs.sort_by_key(|j| if j.method == "core.stop" { 0 } else { 1 });
        for job in jobs {
            if job.binding != self.proxies.binding
                || job.expires_at <= now()
                || !["queued", "executing"].contains(&job.status.as_str())
            {
                continue;
            }
            if let Some(result) = self.proxies.receipts.get(&job.id) {
                let _ = self
                    .send_control(
                        json!({"job_id":job.id,"status":result["status"],"result":result}),
                    )
                    .await;
                continue;
            }
            if self.proxies.receipts.len() >= 128 {
                self.proxies
                    .receipts
                    .retain(|_, v| v["expires_at"].as_u64().unwrap_or(0) > now());
            }
            ensure!(self.proxies.receipts.len() < 128, "receipt journal full");
            self.proxies.receipts.insert(
                job.id.clone(),
                json!({"status":"executing","expires_at":job.expires_at}),
            );
            self.save_proxies().await?;
            // Cloud claim confirms binding is still current before a runtime action.
            if self
                .send_control(json!({"job_id":job.id,"status":"executing"}))
                .await
                .is_err()
            {
                self.proxies.receipts.remove(&job.id);
                self.save_proxies().await?;
                continue;
            }
            if job.method == "proxies.delay" {
                if job.expected_revision.as_deref()
                    != self.cached.as_ref().map(|c| c.revision.as_str())
                {
                    let receipt = json!({"status":"failed","error":"configuration_changed; request a new measurement","expires_at":job.expires_at});
                    self.proxies
                        .receipts
                        .insert(job.id.clone(), receipt.clone());
                    self.save_proxies().await?;
                    let _ = self
                        .send_control(json!({"job_id":job.id,"status":"failed","result":receipt}))
                        .await;
                    continue;
                }
                let http = self.http.clone();
                let core = self.core("/proxies/");
                let secret = self.secret.clone();
                let local = self.local.clone();
                let binding = self.proxies.binding.clone();
                tokio::spawn(async move {
                    let result=async{
                        let mut u=url::Url::parse(&core)?;u.path_segments_mut().unwrap().pop_if_empty().push(job.params["name"].as_str().unwrap_or("")).push("delay");
                        let v:Value=http.get(u).bearer_auth(secret).query(&[("url","https://www.gstatic.com/generate_204"),("timeout","3000")]).timeout(Duration::from_secs(4)).send().await?.error_for_status()?.json().await?;
                        Ok::<_,anyhow::Error>(json!({"name":job.params["name"],"delay":v["delay"],"sampled_at":now()}))
                    }.await;
                    local.results.lock().await.push((job.id,json!({"binding":binding,"status":if result.is_ok(){"succeeded"}else{"failed"},"value":result.ok(),"expires_at":job.expires_at})));
                });
                continue;
            }
            let result = match job.method.as_str() {
                "core.start" | "core.stop" | "core.restart" => self
                    .control_core(job.method.strip_prefix("core.").unwrap(), None)
                    .await
                    .map(|_| json!({})),
                "core.status" => {
                    self.publish_runtime(None).await;
                    Ok(json!({"core_state":self.local.status.lock().await["core_state"]}))
                }
                "proxies.list" => self
                    .local_proxy("proxies.list", Value::Null)
                    .await
                    .map(|_| json!({"snapshot":"reported"})),
                _ => Err(anyhow::anyhow!("unsupported method")),
            };
            let receipt = match result {
                Ok(value) => {
                    json!({"status":"succeeded","value":value,"expires_at":job.expires_at})
                }
                Err(_) => {
                    json!({"status":"failed","error":"device operation failed; inspect local status","expires_at":job.expires_at})
                }
            };
            self.proxies
                .receipts
                .insert(job.id.clone(), receipt.clone());
            self.save_proxies().await?;
            let _ = self
                .send_control(json!({"job_id":job.id,"status":receipt["status"],"result":receipt}))
                .await;
        }
        self.flush_results().await;
        Ok(())
    }
    pub(super) async fn flush_results(&mut self) {
        let completed = std::mem::take(&mut *self.local.results.lock().await);
        for (id, result) in completed {
            if result["binding"] != self.proxies.binding {
                continue;
            }
            self.proxies.receipts.insert(id.clone(), result.clone());
            if self.save_proxies().await.is_ok() {
                let _ = self
                    .send_control(json!({"job_id":id,"status":result["status"],"result":result}))
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn interrupted_rpc_is_unknown_not_replayed() {
        let dir =
            std::env::temp_dir().join(format!("camofy-control-journal-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir(&dir).await.unwrap();
        let d = Durable {
            receipts: [(
                "restart".into(),
                json!({"status":"executing","expires_at":now()+600}),
            )]
            .into(),
            ..Default::default()
        };
        atomic(&dir.join("proxies.json"), &serde_json::to_vec(&d).unwrap())
            .await
            .unwrap();
        assert_eq!(
            Durable::load(&dir).await.unwrap().receipts["restart"]["status"],
            "unknown"
        );
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
    #[test]
    fn legacy_device_overrides_cannot_mask_identity_selection() {
        let mut d = Durable {
            selections: [
                ("a".into(), "identity".into()),
                ("b".into(), "other".into()),
            ]
            .into(),
            overrides: [("a".into(), "cloud-device".into())].into(),
            ..Default::default()
        };
        d.pending.insert("a".into(), Some("offline".into()));
        assert_eq!(d.effective()["a"], "identity");
        d.pending.insert("a".into(), None);
        assert_eq!(d.effective()["a"], "identity");
        assert_eq!(d.effective()["b"], "other");
    }
}
