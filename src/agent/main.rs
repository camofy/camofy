//! Lightweight configuration consumer with a local, first-time authorization UI.
mod application;
mod control;
mod pairing;
mod proxies;
mod shutdown;
use anyhow::{Context, Result, ensure};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::process::{Child, Command};

#[derive(Clone, Deserialize)]
struct Settings {
    #[serde(default)]
    subscription_url: String,
    #[serde(default)]
    cloud_url: String,
    #[serde(default, rename = "device_token")]
    token: String,
    mihomo: PathBuf,
    data_dir: PathBuf,
    #[serde(default)]
    local_overlay: Option<PathBuf>,
    #[serde(default = "default_controller")]
    controller_port: u16,
    #[serde(default)]
    dns_redirect: bool,
    #[serde(default = "default_web_listen")]
    web_listen: Option<String>,
}
fn default_web_listen() -> Option<String> {
    Some("0.0.0.0:3000".into())
}
fn default_controller() -> u16 {
    9091
}
impl Settings {
    fn bound(&self) -> bool {
        !self.token.is_empty() || !self.subscription_url.is_empty()
    }
    fn resolve_subscription(&mut self) -> Result<()> {
        if !self.token.is_empty() {
            let u = url::Url::parse(&self.cloud_url)?;
            ensure!(
                self.token.len() == 64 && self.token.bytes().all(|c| c.is_ascii_hexdigit()),
                "invalid device credential"
            );
            ensure!(
                u.username().is_empty()
                    && u.password().is_none()
                    && u.path() == "/"
                    && u.query().is_none()
                    && u.fragment().is_none(),
                "invalid cloud origin"
            );
            ensure!(
                u.scheme() == "https"
                    || (u.scheme() == "http"
                        && ["localhost", "127.0.0.1", "[::1]"]
                            .contains(&u.host_str().unwrap_or(""))),
                "cloud requires HTTPS"
            );
            self.cloud_url = u.origin().ascii_serialization();
            return Ok(());
        }
        let u = url::Url::parse(&self.subscription_url)?;
        ensure!(
            u.scheme() == "https"
                || (u.scheme() == "http"
                    && ["localhost", "127.0.0.1", "[::1]"].contains(&u.host_str().unwrap_or(""))),
            "subscription URL requires HTTPS"
        );
        ensure!(
            u.username().is_empty()
                && u.password().is_none()
                && u.query().is_none()
                && u.fragment().is_none(),
            "subscription URL must not contain userinfo/query/fragment"
        );
        let parts: Vec<_> = u
            .path_segments()
            .context("invalid subscription URL")?
            .collect();
        ensure!(
            (parts.len() == 2 || (parts.len() == 3 && parts[2] == "router"))
                && parts[0] == "sub"
                && parts[1].len() == 64
                && parts[1].bytes().all(|c| c.is_ascii_hexdigit()),
            "expected a Camofy /sub/<token> subscription URL"
        );
        self.token = parts[1].into();
        self.cloud_url = u.origin().ascii_serialization();
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Cached {
    revision: String,
    hash: String,
    content: String,
    selections: Value,
}
struct Agent {
    s: Settings,
    http: reqwest::Client,
    secret: String,
    child: Option<Child>,
    cached: Option<Cached>,
    command: Option<String>,
    control: control::Durable,
    local: control::Local,
    proxies: proxies::Durable,
    application: application::State,
}
impl Agent {
    async fn publish_runtime(&mut self, error: Option<String>) {
        if self
            .child
            .as_mut()
            .is_some_and(|c| c.try_wait().ok().flatten().is_some())
        {
            self.child = None;
        }
        let state = if self.control.stopped && self.child.is_some() {
            "stopping"
        } else if self.control.stopped {
            "stopped"
        } else if self.child.is_some() {
            "running"
        } else {
            "unavailable"
        };
        let mut status = self.local.status.lock().await;
        status["core_state"] = json!(state);
        status["revision"] = json!(self.cached.as_ref().map(|c| &c.revision));
        status["last_successful_revision"] = status["revision"].clone();
        status["attempted_revision"] = json!(self.application.attempted_revision);
        status["diagnostic"] = json!(self.application.diagnostic);
        status["retry"] = json!(self.application.retry);
        status["error"] = json!(
            error
                .or_else(|| self
                    .application
                    .diagnostic
                    .as_ref()
                    .map(ToString::to_string))
                .or_else(|| self.control.command_error.clone())
        );
    }
    async fn halt(&mut self) -> Result<()> {
        // Remove only the Agent-owned redirect; never flush unrelated firewall state.
        if self.s.dns_redirect
            && let Ok(runtime) =
                tokio::fs::read_to_string(self.s.data_dir.join("running.yaml")).await
        {
            dns_redirect(false, &runtime).await?;
        }
        if let Some(child) = &mut self.child {
            shutdown::terminate(child).await?;
            self.child = None;
        }
        Ok(())
    }
    async fn control_core(&mut self, action: &str, command_id: Option<&str>) -> Result<()> {
        ensure!(
            ["start", "stop", "restart"].contains(&action),
            "unknown action"
        );
        let durable = control::Durable {
            stopped: action == "stop",
            command_id: command_id
                .map(str::to_owned)
                .or_else(|| self.control.command_id.clone()),
            command_error: None,
        };
        atomic(
            &self.s.data_dir.join("control.json"),
            &serde_json::to_vec(&durable)?,
        )
        .await?;
        self.control = durable;
        if action != "stop" {
            // An explicit operator start/restart is a deliberate retry, independent
            // of the automatic candidate/recovery cooldown.
            self.application.manual_retry();
        }
        let result = async {
            if action != "start" {
                self.halt().await?;
            }
            if action != "stop" {
                let cache = self
                    .cached
                    .clone()
                    .context("尚无有效配置，请等待身份下发后再启动")?;
                self.apply(cache).await?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        self.control.command_error = result.as_ref().err().map(|e| e.to_string());
        atomic(
            &self.s.data_dir.join("control.json"),
            &serde_json::to_vec(&self.control)?,
        )
        .await?;
        self.publish_runtime(result.as_ref().err().map(|e| e.to_string()))
            .await;
        // Do not leave a stopped/empty snapshot until the next watchdog tick.
        self.reconcile_proxies().await;
        result
    }
    fn api(&self, path: &str) -> String {
        format!("{}{}", self.s.cloud_url.trim_end_matches('/'), path)
    }
    fn core(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.s.controller_port)
    }
    async fn report(&self, _revision: Option<&str>, status: &str, message: &str, extra: Value) {
        let revision = self.cached.as_ref().map(|c| c.revision.as_str());
        let status = if status == "online" && self.application.diagnostic.is_some() {
            "failed"
        } else {
            status
        };
        let mut body = json!({"revision":revision,"last_successful_revision":revision,
            "attempted_revision":self.application.attempted_revision,
            "diagnostic":self.application.diagnostic,"retry":self.application.retry,
            "status":status,"message":message});
        if let Some(extra) = extra.as_object() {
            for (k, v) in extra {
                body[k] = v.clone();
            }
        }
        let _ = self
            .http
            .post(self.api("/api/sync/report"))
            .bearer_auth(&self.s.token)
            .json(&body)
            .send()
            .await;
    }
    async fn runtime(&self, content: &str) -> Result<String> {
        let mut config = camofy::engine::parse(content)?;
        if let Some(path) = &self.s.local_overlay {
            let overlay = tokio::fs::read_to_string(path).await?;
            camofy::engine::merge(&mut config, &camofy::engine::parse(&overlay)?)?;
        }
        // Local port/DNS overrides must not remove the cloud's safety policy.
        let safety = camofy::engine::cloud_safety_profile(&self.s.cloud_url)?;
        let rule = safety["prepend-rules"][0].clone();
        if let Some(rules) = config
            .get_mut("rules")
            .and_then(serde_yaml::Value::as_sequence_mut)
        {
            rules.retain(|r| *r != rule);
        }
        camofy::engine::merge(&mut config, &safety)?;
        let map = config.as_mapping_mut().unwrap();
        for key in [
            "external-controller-unix",
            "external-controller-pipe",
            "external-controller-tls",
            "external-ui",
            "external-ui-url",
        ] {
            map.remove(key);
        }
        map.insert(
            "external-controller".into(),
            format!("127.0.0.1:{}", self.s.controller_port).into(),
        );
        map.insert("secret".into(), self.secret.clone().into());
        camofy::engine::validate(&config)?;
        Ok(serde_yaml::to_string(&config)?)
    }
    async fn load_core(&mut self, path: &Path) -> Result<()> {
        if let Some(child) = &mut self.child
            && child.try_wait()?.is_some()
        {
            self.child = None;
        }
        if self.child.is_some() {
            let response = self
                .http
                .put(self.core("/configs"))
                .bearer_auth(&self.secret)
                .query(&[("force", "true")])
                .json(&json!({"path":path}))
                .send()
                .await
                .map_err(|e| application::Diagnostic::safe("core_reload", &e.into()))?;
            if !response.status().is_success() {
                return Err(application::Diagnostic::new(
                    "core_reload",
                    "reload_rejected",
                    &format!(
                        "Mihomo rejected configuration reload (HTTP {})",
                        response.status().as_u16()
                    ),
                )
                .into());
            }
        } else {
            self.child = Some(
                Command::new(&self.s.mihomo)
                    .arg("-d")
                    .arg(&self.s.data_dir)
                    .arg("-f")
                    .arg(path)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(false)
                    .spawn()
                    .map_err(|e| application::Diagnostic::safe("core_start", &e.into()))?,
            );
        }
        for _ in 0..30 {
            if let Some(child) = &mut self.child
                && let Some(status) = child
                    .try_wait()
                    .map_err(|e| application::Diagnostic::safe("health", &e.into()))?
            {
                let mut diagnostic = application::Diagnostic::rejected(
                    status,
                    &Default::default(),
                    &Default::default(),
                );
                diagnostic.stage = "health".into();
                diagnostic.kind = "core_exited".into();
                diagnostic.message = "Mihomo exited before its controller became healthy".into();
                self.child = None;
                return Err(diagnostic.into());
            }
            if self
                .http
                .get(self.core("/version"))
                .bearer_auth(&self.secret)
                .timeout(Duration::from_secs(1))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Err(application::Diagnostic::new(
            "health",
            "core_unavailable",
            "Mihomo controller did not become healthy after configuration application",
        )
        .into())
    }
    async fn selections(&self, selections: &Value) -> Result<()> {
        for (group, node) in selections.as_object().into_iter().flatten() {
            let mut u = url::Url::parse(&self.core("/proxies/"))?;
            u.path_segments_mut().unwrap().pop_if_empty().push(group);
            self.http
                .put(u)
                .bearer_auth(&self.secret)
                .json(&json!({"name":node}))
                .send()
                .await?
                .error_for_status()?;
        }
        Ok(())
    }
    async fn input_key(&self, _revision: &str, hash: &str) -> Result<String> {
        // Revisions can change without changing the artifact (for example a
        // selection-only update). Such updates must not trigger another core
        // validation process alongside a running core.
        let mut inputs = format!(
            "{hash}:{}:{}:{}:{}",
            self.s.controller_port, self.s.dns_redirect, self.control.stopped, self.s.cloud_url
        );
        if let Some(path) = &self.s.local_overlay {
            inputs.push_str(&camofy::digest(&tokio::fs::read(path).await?));
        }
        let mut paths = vec![self.s.mihomo.clone()];
        let mut files = tokio::fs::read_dir(&self.s.data_dir).await?;
        while let Some(file) = files.next_entry().await? {
            let path = file.path();
            if path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| ["dat", "mmdb", "mrs"].contains(&x.to_ascii_lowercase().as_str()))
            {
                paths.push(path);
            }
        }
        paths.sort();
        for path in paths {
            let meta = tokio::fs::metadata(&path).await?;
            inputs.push_str(&format!(
                "{}:{}:{:?}",
                path.display(),
                meta.len(),
                meta.modified()?
            ));
        }
        Ok(camofy::digest(&inputs))
    }
    async fn save_application(&self) {
        if atomic(
            &self.s.data_dir.join("apply-state.json"),
            &serde_json::to_vec(&self.application).unwrap(),
        )
        .await
        .is_err()
        {
            tracing::warn!("could not persist application retry state");
        }
    }
    async fn apply(&mut self, cache: Cached) -> Result<()> {
        let key = self
            .input_key(&cache.revision, &cache.hash)
            .await
            .map_err(|e| application::Diagnostic::safe("prepare", &e))?;
        if let Some(diagnostic) = self.application.blocked(&key, now()) {
            return Err(diagnostic.into());
        }
        let revision = cache.revision.clone();
        let result = self.apply_inner(cache).await;
        match &result {
            Ok(()) => self.application.succeeded(key, &revision),
            Err(error) => {
                let diagnostic = application::Diagnostic::safe("application", error);
                tracing::warn!(%diagnostic, attempted_revision=%revision, "candidate application failed; keeping last good configuration");
                self.application.failed(key, &revision, diagnostic, now());
            }
        }
        self.save_application().await;
        result.map_err(|e| application::Diagnostic::safe("application", &e).into())
    }
    async fn validate_candidate(&self, candidate: &Path) -> Result<()> {
        let mut child = Command::new(&self.s.mihomo)
            .arg("-t")
            .arg("-d")
            .arg(&self.s.data_dir)
            .arg("-f")
            .arg(candidate)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| application::Diagnostic::safe("validation", &e.into()))?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            tokio::try_join!(
                child.wait(),
                application::capture(stdout),
                application::capture(stderr)
            )
        })
        .await;
        match result {
            Ok(Ok((status, stdout, stderr))) if !status.success() => {
                Err(application::Diagnostic::rejected(status, &stdout, &stderr).into())
            }
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => {
                Err(application::Diagnostic::safe("validation", &error.into()).into())
            }
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                Err(application::Diagnostic::new(
                    "validation",
                    "timeout",
                    "Mihomo validation exceeded 30 seconds",
                )
                .into())
            }
        }
    }
    async fn apply_inner(&mut self, cache: Cached) -> Result<()> {
        ensure!(
            camofy::digest(&cache.content) == cache.hash,
            "artifact hash mismatch"
        );
        let runtime = self
            .runtime(&cache.content)
            .await
            .map_err(|e| application::Diagnostic::safe("prepare", &e))?;
        let candidate = self.s.data_dir.join("candidate.yaml");
        atomic(&candidate, runtime.as_bytes())
            .await
            .map_err(|e| application::Diagnostic::safe("candidate_write", &e))?;
        self.validate_candidate(&candidate).await?;
        let running = self.s.data_dir.join("running.yaml");
        let previous = tokio::fs::read(&running).await.ok();
        atomic(&running, runtime.as_bytes())
            .await
            .map_err(|e| application::Diagnostic::safe("runtime_write", &e))?;
        let result = async {
            if !self.control.stopped {
                self.load_core(&running).await?;
                // Selection failures are reported separately and never roll back valid YAML.
                let selections = if self.proxies.binding.is_empty() {
                    cache.selections.clone()
                } else {
                    json!(self.proxies.effective())
                };
                let _ = self.selections(&selections).await;
            }
            if self.s.dns_redirect && !self.control.stopped {
                dns_redirect(true, &runtime)
                    .await
                    .map_err(|e| application::Diagnostic::safe("dns_redirect", &e))?;
            }
            atomic(
                &self.s.data_dir.join("last-good.json"),
                &serde_json::to_vec(&cache)?,
            )
            .await
            .map_err(|e| application::Diagnostic::safe("durable_write", &e))?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(e) = result {
            if let Some(old) = previous {
                atomic(&running, &old).await?;
                if !self.control.stopped && self.load_core(&running).await.is_err() {
                    self.halt()
                        .await
                        .context("cannot gracefully stop failed core")?;
                    self.load_core(&running).await.context("rollback failed")?;
                }
                if !self.control.stopped
                    && let Some(old) = &self.cached
                {
                    let _ = self.selections(&old.selections).await;
                }
                if self.s.dns_redirect && !self.control.stopped {
                    dns_redirect(true, std::str::from_utf8(&old)?)
                        .await
                        .context("DNS rollback failed")?;
                }
            } else {
                self.halt()
                    .await
                    .context("cannot gracefully stop rejected core")?;
                if self.s.dns_redirect {
                    let _ = dns_redirect(false, &runtime).await;
                }
            }
            return Err(e);
        }
        // Mark current only after core, selections, DNS and durable cache all succeeded.
        self.cached = Some(cache);
        Ok(())
    }
    async fn sync(&mut self) -> Result<()> {
        let desired: Value = self
            .http
            .get(self.api("/api/sync/desired"))
            .bearer_auth(&self.s.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let revision = desired["revision"].as_str().context("missing revision")?;
        self.accept_control(&desired["control"]).await?;
        let jobs_control_requested = desired["control"]["jobs"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|job| {
                ["core.start", "core.stop", "core.restart"]
                    .contains(&job["method"].as_str().unwrap_or(""))
                    && job["id"]
                        .as_str()
                        .is_some_and(|id| !self.proxies.receipts.contains_key(id))
            });
        // Emergency stop takes precedence over downloads, validation and slow providers.
        let stops: Vec<Value> = desired["control"]["jobs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|j| j["method"] == "core.stop")
            .cloned()
            .collect();
        self.process_jobs(&json!(stops)).await?;
        self.local.status.lock().await["identity_name"] = desired["identity_name"].clone();
        // Stop is handled before downloading/applying anything, including on first sync.
        let cmd = &desired["command"];
        let pending = cmd["id"].as_str().filter(|id| {
            self.control.command_id.as_deref() != Some(*id)
                && cmd["expires_at"].as_u64().unwrap_or(0) > now()
        });
        let control_requested = jobs_control_requested
            || (pending.is_some()
                && ["start", "stop", "restart"].contains(&cmd["type"].as_str().unwrap_or("")));
        if cmd["type"] == "stop"
            && let Some(id) = pending
        {
            let result = self.control_core("stop", Some(id)).await;
            self.publish_runtime(result.as_ref().err().map(|e| e.to_string()))
                .await;
            self.report(None, "online", "core control completed", json!({"command_id":id,"core_state":self.local.status.lock().await["core_state"],"command_error":result.err().map(|e|e.to_string())})).await;
        }
        if self
            .child
            .as_mut()
            .is_some_and(|c| c.try_wait().ok().flatten().is_some())
        {
            self.child = None;
        }
        if self.child.is_none()
            && !self.control.stopped
            && let Some(cache) = self.cached.clone()
        {
            // Recovery cooldown must not prevent command processing or a newer
            // candidate from being attempted in this same sync.
            let _ = self.apply(cache).await;
        }
        let control_v2 = desired["control"]["protocol"] == 2;
        let hash = if control_v2 {
            &desired["control"]["hash"]
        } else {
            &desired["hash"]
        };
        let format = if control_v2 {
            desired["control"]["artifact"].as_str().unwrap_or("router")
        } else {
            "router"
        };
        let hash = hash.as_str().context("missing hash")?;
        let inputs = self.input_key(revision, hash).await;
        let key = inputs
            .as_ref()
            .cloned()
            .unwrap_or_else(|_| camofy::digest(format!("{revision}:{hash}:inputs-unavailable")));
        let mut candidate_failure = None;
        if self.cached.as_ref().is_none_or(|c| c.hash != hash)
            || self.application.applied_inputs.as_deref() != Some(&key)
        {
            let attempt = if let Some(error) = self.application.blocked(&key, now()) {
                Err(error.into())
            } else if let Err(error) = inputs {
                let diagnostic = application::Diagnostic::safe("prepare", &error);
                self.application
                    .failed(key.clone(), revision, diagnostic.clone(), now());
                Err(diagnostic.into())
            } else {
                async {
                    let mut response = self
                        .http
                        .get(self.api(&format!("/api/sync/revisions/{revision}/{format}")))
                        .bearer_auth(&self.s.token)
                        .send()
                        .await?
                        .error_for_status()?;
                    ensure!(
                        response
                            .headers()
                            .get("x-camofy-revision")
                            .and_then(|h| h.to_str().ok())
                            == Some(revision),
                        "subscription changed during sync; retry with latest desired state"
                    );
                    let mut bytes = Vec::new();
                    while let Some(chunk) = response.chunk().await? {
                        ensure!(
                            bytes.len() + chunk.len() <= 4 * 1024 * 1024,
                            "artifact too large"
                        );
                        bytes.extend(chunk);
                    }
                    let cache = Cached {
                        revision: revision.into(),
                        hash: hash.into(),
                        content: String::from_utf8(bytes)?,
                        selections: desired["selections"].clone(),
                    };
                    self.apply(cache).await
                }
                .await
            };
            if let Err(error) = attempt {
                let diagnostic = application::Diagnostic::safe("download", &error);
                // apply() recorded validation/application failures; also back off
                // failed downloads without losing their safe stage information.
                if error.downcast_ref::<application::Diagnostic>().is_none() {
                    self.application
                        .failed(key.clone(), revision, diagnostic.clone(), now());
                    self.save_application().await;
                }
                candidate_failure = Some((diagnostic, self.application.retry.clone()));
                self.application.pending_candidate =
                    Some((revision.into(), hash.into(), key.clone()));
                self.save_application().await;
            }
        }
        if candidate_failure.is_none() && self.application.pending_candidate.take().is_some() {
            self.save_application().await;
        }
        if candidate_failure.is_none()
            && let Some(cache) = &mut self.cached
        {
            // A new revision may reuse already validated content. Accept its
            // identity without another core reload, and clear stale diagnostics.
            self.application.succeeded(key.clone(), revision);
            cache.revision = revision.into();
            cache.selections = desired["selections"].clone();
            atomic(
                &self.s.data_dir.join("last-good.json"),
                &serde_json::to_vec(cache)?,
            )
            .await?;
            self.save_application().await;
        }
        self.reconcile_proxies().await;
        self.process_jobs(&desired["control"]["jobs"]).await?;
        if ["start", "restart"].contains(&cmd["type"].as_str().unwrap_or(""))
            && let Some(id) = pending
        {
            let result = self
                .control_core(cmd["type"].as_str().unwrap(), Some(id))
                .await;
            self.publish_runtime(result.as_ref().err().map(|e| e.to_string()))
                .await;
            self.report(Some(revision), "online", "core control completed", json!({"command_id":id,"core_state":self.local.status.lock().await["core_state"],"command_error":result.err().map(|e|e.to_string())})).await;
        } else if let Some(id) = cmd["id"].as_str()
            && self.control.command_id.as_deref() == Some(id)
        {
            // Re-send a lost acknowledgement without repeating a restart.
            self.publish_runtime(None).await;
            self.report(
                Some(revision),
                "online",
                "core control acknowledged",
                json!({"command_id":id,"core_state":self.local.status.lock().await["core_state"],"command_error":self.control.command_error}),
            )
            .await;
        }
        self.publish_runtime(None).await;
        let runtime_state = self.local.status.lock().await["core_state"].clone();
        let final_failure = candidate_failure
            .or_else(|| {
                self.application
                    .diagnostic
                    .clone()
                    .map(|diagnostic| (diagnostic, self.application.retry.clone()))
            })
            .or_else(|| {
                if !control_requested {
                    return None;
                }
                self.control.command_error.as_ref().map(|error| {
                    (
                        application::Diagnostic::safe("core_control", &anyhow::anyhow!("{error}")),
                        None,
                    )
                })
            })
            .or_else(|| {
                let expected = if self.control.stopped {
                    "stopped"
                } else {
                    "running"
                };
                (runtime_state != expected).then(|| {
                    (
                        application::Diagnostic::new(
                            "health",
                            "core_unavailable",
                            "Mihomo has not reached the requested running or stopped state",
                        ),
                        None,
                    )
                })
            });
        if let Some((diagnostic, retry)) = final_failure {
            self.application.attempted_revision = Some(revision.into());
            self.application.diagnostic = Some(diagnostic.clone());
            self.application.retry = retry;
            self.save_application().await;
            self.publish_runtime(Some(diagnostic.to_string())).await;
            self.report(
                None,
                "failed",
                &diagnostic.to_string(),
                json!({"core_state":self.local.status.lock().await["core_state"],"protocol":2}),
            )
            .await;
        } else {
            self.publish_runtime(None).await;
            self.report(
                Some(revision),
                "applied",
                if self.control.stopped {
                    "configuration saved; core stopped"
                } else {
                    "configuration active"
                },
                json!({"core_state":self.local.status.lock().await["core_state"],"protocol":2}),
            )
            .await;
        }
        if let Some(id) = desired["command"]["id"].as_str()
            && self.command.as_deref() != Some(id)
            && desired["command"]["type"] == "test_delays"
            && desired["command"]["expires_at"].as_u64().unwrap_or(0) > now()
        {
            self.command = Some(id.into());
            let worker = Agent {
                s: self.s.clone(),
                http: self.http.clone(),
                secret: self.secret.clone(),
                child: None,
                cached: self.cached.clone(),
                command: None,
                control: self.control.clone(),
                local: self.local.clone(),
                proxies: self.proxies.clone(),
                application: self.application.clone(),
            };
            let id = id.to_string();
            // A slow node must not block realtime config application or the watchdog.
            tokio::spawn(async move {
                if let Ok(delays) = worker.test_delays().await {
                    // The main agent reports with its current successful revision
                    // and failure state, not this slow worker's old snapshot.
                    let _ = worker
                        .local
                        .tx
                        .send(control::Request::DelayReport { id, delays })
                        .await;
                }
            });
        }
        Ok(())
    }
    async fn test_delays(&self) -> Result<Value> {
        let proxies: Value = self
            .http
            .get(self.core("/proxies"))
            .bearer_auth(&self.secret)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut result = serde_json::Map::new();
        for (name, p) in proxies["proxies"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, p)| p.get("all").is_none())
            .take(32)
        {
            if ["Direct", "Reject"].contains(&p["type"].as_str().unwrap_or("")) {
                continue;
            }
            let mut u = url::Url::parse(&self.core("/proxies/"))?;
            u.path_segments_mut()
                .unwrap()
                .pop_if_empty()
                .push(name)
                .push("delay");
            u.query_pairs_mut()
                .append_pair("url", "https://www.gstatic.com/generate_204")
                .append_pair("timeout", "3000");
            let value = match self
                .http
                .get(u)
                .bearer_auth(&self.secret)
                .timeout(Duration::from_secs(4))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => {
                    r.json::<Value>().await.unwrap_or(json!({"delay":null}))
                }
                _ => json!({"delay":null}),
            };
            result.insert(name.clone(), value["delay"].clone());
        }
        Ok(Value::Object(result))
    }
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
async fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let tmp = path.with_extension("tmp");
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).await?;
    file.write_all(bytes).await?;
    file.sync_all().await?;
    drop(file);
    tokio::fs::rename(tmp, path).await?;
    Ok(())
}
async fn dns_redirect(enable: bool, runtime: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let v = camofy::engine::parse(runtime)?;
        let address: std::net::SocketAddr = v["dns"]["listen"]
            .as_str()
            .context("DNS listen required for redirect")?
            .parse()?;
        let port = address.port().to_string();
        let _ = Command::new("iptables")
            .args(["-t", "nat", "-N", "CAMOFY_DNS"])
            .status()
            .await;
        for proto in ["udp", "tcp"] {
            if enable {
                let _ = Command::new("iptables")
                    .args(["-t", "nat", "-F", "CAMOFY_DNS"])
                    .status()
                    .await;
                // Flush once below and install both transports together.
                break;
            } else {
                let _ = Command::new("iptables")
                    .args([
                        "-t",
                        "nat",
                        "-D",
                        "PREROUTING",
                        "-p",
                        proto,
                        "--dport",
                        "53",
                        "-j",
                        "CAMOFY_DNS",
                    ])
                    .status()
                    .await;
            }
        }
        if enable {
            let status = Command::new("iptables")
                .args([
                    "-t",
                    "nat",
                    "-A",
                    "CAMOFY_DNS",
                    "-j",
                    "REDIRECT",
                    "--to-ports",
                    &port,
                ])
                .status()
                .await?;
            ensure!(status.success(), "cannot configure DNS redirect");
            for proto in ["udp", "tcp"] {
                if !Command::new("iptables")
                    .args([
                        "-t",
                        "nat",
                        "-C",
                        "PREROUTING",
                        "-p",
                        proto,
                        "--dport",
                        "53",
                        "-j",
                        "CAMOFY_DNS",
                    ])
                    .status()
                    .await?
                    .success()
                {
                    ensure!(
                        Command::new("iptables")
                            .args([
                                "-t",
                                "nat",
                                "-A",
                                "PREROUTING",
                                "-p",
                                proto,
                                "--dport",
                                "53",
                                "-j",
                                "CAMOFY_DNS"
                            ])
                            .status()
                            .await?
                            .success(),
                        "cannot attach DNS redirect"
                    );
                }
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (enable, runtime);
    }
    Ok(())
}
async fn notifications(s: Settings, tx: tokio::sync::watch::Sender<u64>) {
    let mut backoff = 1u64;
    loop {
        let result = async {
            let url = format!("{}/api/sync/ws", s.cloud_url.trim_end_matches('/'))
                .replacen("https://", "wss://", 1)
                .replacen("http://", "ws://", 1);
            let (mut ws, _) = tokio::time::timeout(
                Duration::from_secs(15),
                tokio_tungstenite::connect_async(url),
            )
            .await??;
            ws.send(tokio_tungstenite::tungstenite::Message::Text(
                json!({"token":s.token}).to_string().into(),
            ))
            .await?;
            loop {
                let msg = tokio::time::timeout(Duration::from_secs(90), ws.next())
                    .await?
                    .context("websocket closed")??;
                match msg {
                    tokio_tungstenite::tungstenite::Message::Text(_) => {
                        backoff = 1;
                        tx.send_modify(|n| *n += 1);
                    }
                    tokio_tungstenite::tungstenite::Message::Ping(p) => {
                        ws.send(tokio_tungstenite::tungstenite::Message::Pong(p))
                            .await?
                    }
                    tokio_tungstenite::tungstenite::Message::Close(_) => break,
                    _ => {}
                }
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if result.is_err() {
            tracing::debug!("notification connection unavailable; polling remains active");
        }
        let jitter = (uuid::Uuid::new_v4().as_u128() % 1000) as u64;
        tokio::time::sleep(Duration::from_millis(backoff * 1000 + jitter)).await;
        backoff = (backoff * 2).min(60);
    }
}

// Register the Unix signal handler eagerly, before the returned future is polled.
#[allow(clippy::manual_async_fn)]
fn shutdown() -> impl std::future::Future<Output = ()> {
    #[cfg(unix)]
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install SIGTERM handler");
    async move {
        #[cfg(unix)]
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let shutdown_requested = shutdown();
    tokio::pin!(shutdown_requested);
    let path = std::env::args()
        .nth(1)
        .context("usage: camofy-agent /path/to/agent.json")?;
    let mut s: Settings = serde_json::from_slice(&tokio::fs::read(&path).await?)?;
    if s.bound() {
        s.resolve_subscription()?;
    }
    tokio::fs::create_dir_all(&s.data_dir).await?;
    s.data_dir = tokio::fs::canonicalize(&s.data_dir).await?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(s.data_dir.join("agent.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock).context("another agent owns this data directory")?;
    if !s.subscription_url.is_empty() {
        // One-time lossless upgrade for previously installed devices, no user action.
        let mut saved: Value = serde_json::from_slice(&tokio::fs::read(&path).await?)?;
        saved
            .as_object_mut()
            .context("settings must be an object")?
            .remove("subscription_url");
        saved["cloud_url"] = json!(s.cloud_url);
        saved["device_token"] = json!(s.token);
        atomic(Path::new(&path), &serde_json::to_vec_pretty(&saved)?).await?;
        s.subscription_url.clear();
    }
    let (local, mut controls) = control::channel();
    let mut binding = pairing::start(&s, PathBuf::from(&path), local.clone()).await?;
    if !s.bound() {
        tracing::info!("Agent is unbound; open the local binding page to authorize it");
        loop {
            tokio::select! {
                result = binding.changed() => {
                    result.context("binding service stopped")?;
                    let authorized = binding.borrow().is_some();
                    if authorized {
                        let saved: Settings = serde_json::from_slice(&tokio::fs::read(&path).await?)?;
                        s.cloud_url = saved.cloud_url;
                        s.token = saved.token;
                        break;
                    }
                },
                _ = &mut shutdown_requested => return Ok(()),
            }
        }
        s.resolve_subscription()?;
    }
    s.mihomo = tokio::fs::canonicalize(&s.mihomo)
        .await
        .context("Mihomo must already be installed")?;
    let mut agent = Agent {
        s: s.clone(),
        http: reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()?,
        secret: uuid::Uuid::new_v4().to_string(),
        child: None,
        cached: None,
        command: None,
        control: match tokio::fs::read(s.data_dir.join("control.json")).await {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).context("invalid persistent core control state")?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
            Err(e) => return Err(e.into()),
        },
        local,
        proxies: proxies::Durable::load(&s.data_dir).await?,
        application: match tokio::fs::read(s.data_dir.join("apply-state.json")).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Default::default(),
        },
    };
    if let Ok(bytes) = tokio::fs::read(s.data_dir.join("last-good.json")).await {
        let cache: Cached = serde_json::from_slice(&bytes)?;
        // Retain the known successful revision even if restoring its runtime
        // fails. The agent must remain online so stop/new config can recover it.
        agent.cached = Some(cache.clone());
        // A newly generated controller secret requires a fresh runtime file on
        // restart. Failed *candidate* cooldowns remain persisted independently.
        if let Err(error) = agent.apply_inner(cache.clone()).await {
            let diagnostic = application::Diagnostic::safe("restore", &error);
            tracing::warn!(%diagnostic, "cannot restore last good configuration");
            if let Ok(key) = agent.input_key(&cache.revision, &cache.hash).await {
                agent
                    .application
                    .failed(key, &cache.revision, diagnostic, now());
                agent.save_application().await;
            }
        } else if let Ok(key) = agent.input_key(&cache.revision, &cache.hash).await {
            // The restore just validated and applied these exact local inputs.
            // Avoid immediately spawning a duplicate validation beside the core,
            // while retaining any different failed candidate's retry history.
            agent.application.restored(key, &cache.revision);
            agent.save_application().await;
        }
    }
    let (tx, mut rx) = tokio::sync::watch::channel(0);
    let notification = tokio::spawn(notifications(s, tx));
    let mut poll = tokio::time::interval(Duration::from_secs(300));
    let mut health = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            Some(request)=controls.recv()=>{
                let action=match request {
                    control::Request::Core(action)=>action,
                    control::Request::DelayReport{id,delays}=>{
                        agent.report(None,"online","device delay test completed",json!({"command_id":id,"delays":delays})).await;
                        continue;
                    }
                    control::Request::Proxy{method,params,reply}=>{
                        let result=agent.local_proxy(&method,params).await.map_err(|e|e.to_string());
                        let _=reply.send(result);
                        if method=="proxies.select" {let _=agent.sync().await;}
                        continue;
                    }
                };
                let result = agent.control_core(&action, None).await;
                let error = result.err().map(|e|e.to_string());
                agent.publish_runtime(error.clone()).await;
                agent.report(None,"online","local core control completed",json!({"core_state":agent.local.status.lock().await["core_state"],"command_error":error})).await;
            },
            _=poll.tick()=>{if let Err(error)=agent.sync().await{let diagnostic=application::Diagnostic::safe("sync",&error);tracing::warn!(%diagnostic,"sync failed; keeping last good configuration");}},
            _=rx.changed()=>{if let Err(error)=agent.sync().await{let diagnostic=application::Diagnostic::safe("sync",&error);tracing::warn!(%diagnostic,"sync failed; five-minute fallback remains active");}},
            _=health.tick()=>{
                agent.flush_results().await;
                agent.reconcile_proxies().await;
                // Cooldown never sleeps the command loop. The watchdog checks
                // eligibility and notices local overlay/core/geodata changes.
                if let Some((revision, hash, failed_key)) = agent.application.pending_candidate.clone() {
                    let changed = agent.input_key(&revision,&hash).await.is_ok_and(|key| key != failed_key);
                    if changed || agent.application.retry_due(&failed_key,now()) {
                        let _ = agent.sync().await;
                    }
                }
                if agent.control.stopped { agent.publish_runtime(None).await; continue; }
                let exited=agent.child.as_mut().is_some_and(|c|c.try_wait().ok().flatten().is_some());
                if exited {
                    agent.child=None;
                    if let Some(c)=agent.cached.clone() {
                        if agent.s.dns_redirect {let _=dns_redirect(false,&c.content).await;}
                        if agent.apply(c).await.is_err(){tracing::warn!("core recovery failed; retrying");}
                    }
                }
                else if agent.child.is_none()&& let Some(c)=agent.cached.clone(){let _=agent.apply(c).await;}
                agent.publish_runtime(None).await;
            },
            _=&mut shutdown_requested=>break,
        }
    }
    notification.abort();
    // Keep ownership until Mihomo confirms exit; never silently SIGKILL a TUN core.
    while let Err(error) = agent.halt().await {
        tracing::error!(%error, "graceful shutdown incomplete; retaining core ownership");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_neutral_and_legacy_identity_urls_only() {
        let base = format!("https://cloud.example/sub/{}", "a".repeat(64));
        for (url, valid) in [
            (base.clone(), true),
            (format!("{base}/router"), true),
            (format!("{base}/clash"), false),
            (format!("{base}/extra/router"), false),
            (format!("{base}?token=other"), false),
            ("https://cloud.example/sub/short".into(), false),
            ("https://cloud.example/".into(), false),
        ] {
            let mut s: Settings = serde_json::from_value(
                json!({"subscription_url":url,"mihomo":"mihomo","data_dir":"data"}),
            )
            .unwrap();
            assert_eq!(s.resolve_subscription().is_ok(), valid);
            if valid {
                assert_eq!(s.cloud_url, "https://cloud.example");
                assert_eq!(s.token, "a".repeat(64));
            }
        }
    }
    #[tokio::test]
    async fn failed_candidate_keeps_successful_revision_and_cooldown_does_not_block_stop() {
        use axum::{
            Json, Router,
            routing::{get, post},
        };
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let root = std::env::temp_dir().join(format!("camofy-retry-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        // An existing non-executable file gives a portable validation spawn failure.
        let core = root.join("invalid-core");
        tokio::fs::write(&core, "not an executable").await.unwrap();
        let content = "proxies: []\nproxy-groups: []\nrules: ['MATCH,DIRECT']\n";
        let desired = Arc::new(tokio::sync::Mutex::new(
            json!({"revision":"candidate", "hash":camofy::digest(content),"selections":{}}),
        ));
        let reports = Arc::new(tokio::sync::Mutex::new(Vec::<Value>::new()));
        let downloads = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route(
                "/api/sync/desired",
                get({
                    let desired = desired.clone();
                    move || {
                        let desired = desired.clone();
                        async move { Json(desired.lock().await.clone()) }
                    }
                }),
            )
            .route(
                "/api/sync/revisions/candidate/router",
                get({
                    let downloads = downloads.clone();
                    move || {
                        let downloads = downloads.clone();
                        async move {
                            downloads.fetch_add(1, Ordering::SeqCst);
                            ([("x-camofy-revision", "candidate")], content)
                        }
                    }
                }),
            )
            .route(
                "/api/sync/report",
                post({
                    let reports = reports.clone();
                    move |Json(body): Json<Value>| {
                        let reports = reports.clone();
                        async move {
                            reports.lock().await.push(body);
                            Json(json!({}))
                        }
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let good = Cached {
            revision: "last-good".into(),
            hash: camofy::digest("previous configuration"),
            content: "previous configuration".into(),
            selections: json!({}),
        };
        let mut agent = Agent {
            s: Settings {
                subscription_url: String::new(),
                cloud_url: origin,
                token: "test".into(),
                mihomo: core,
                data_dir: root.clone(),
                local_overlay: None,
                controller_port: 9,
                dns_redirect: false,
                web_listen: None,
            },
            http: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            secret: "test-controller-secret".into(),
            child: None,
            cached: Some(good.clone()),
            command: None,
            control: control::Durable {
                stopped: true,
                ..Default::default()
            },
            local: control::channel().0,
            proxies: Default::default(),
            application: Default::default(),
        };
        atomic(
            &root.join("last-good.json"),
            &serde_json::to_vec(&good).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(
            agent.input_key("one-revision", "same-hash").await.unwrap(),
            agent
                .input_key("another-revision", "same-hash")
                .await
                .unwrap(),
            "revision-only changes must not revalidate beside a running core"
        );
        agent.sync().await.unwrap();
        assert_eq!(downloads.load(Ordering::SeqCst), 1);
        assert_eq!(agent.cached.as_ref().unwrap().revision, "last-good");
        let first = reports.lock().await.last().unwrap().clone();
        assert_eq!(first["status"], "failed");
        assert_eq!(first["revision"], "last-good");
        assert_eq!(first["last_successful_revision"], "last-good");
        assert_eq!(first["attempted_revision"], "candidate");
        assert_eq!(first["diagnostic"]["stage"], "validation");
        assert_eq!(first["retry"]["failures"], 1);
        assert!(!first.to_string().contains("test-controller-secret"));
        tokio::fs::write(root.join("candidate.yaml"), "unchanged during cooldown")
            .await
            .unwrap();
        desired.lock().await["command"] =
            json!({"id":"stop-during-backoff","type":"stop","expires_at":now()+300});
        agent.sync().await.unwrap();
        assert_eq!(
            agent.control.command_id.as_deref(),
            Some("stop-during-backoff")
        );
        assert_eq!(
            downloads.load(Ordering::SeqCst),
            1,
            "cooldown must skip candidate download and validation"
        );
        assert_eq!(
            tokio::fs::read_to_string(root.join("candidate.yaml"))
                .await
                .unwrap(),
            "unchanged during cooldown"
        );
        let last = reports.lock().await.last().unwrap().clone();
        assert_eq!(last["revision"], "last-good");
        assert_eq!(last["status"], "failed");
        assert_eq!(last["retry"]["failures"], 1);
        assert_eq!(last["core_state"], "stopped");
        let persisted: Cached =
            serde_json::from_slice(&tokio::fs::read(root.join("last-good.json")).await.unwrap())
                .unwrap();
        assert_eq!(persisted.revision, "last-good");
        // A local matcher change bypasses the old failure key immediately.
        let overlay = root.join("overlay.yaml");
        tokio::fs::write(&overlay, "geosite-matcher: mph\n")
            .await
            .unwrap();
        agent.s.local_overlay = Some(overlay);
        agent.sync().await.unwrap();
        assert_eq!(downloads.load(Ordering::SeqCst), 2);
        // The desired artifact is already the successful A, but its core died
        // and recovery fails. A historical applied fingerprint is not health.
        agent.cached = Some(Cached {
            revision: "candidate".into(),
            hash: camofy::digest(content),
            content: content.into(),
            selections: json!({}),
        });
        agent.control = Default::default();
        agent.application = Default::default();
        let active_key = agent
            .input_key("candidate", &camofy::digest(content))
            .await
            .unwrap();
        agent.application.succeeded(active_key, "candidate");
        desired.lock().await["command"] = Value::Null;
        agent.sync().await.unwrap();
        let recovery = reports.lock().await.last().unwrap().clone();
        assert_eq!(
            recovery["status"], "failed",
            "failed same-artifact recovery cannot report active"
        );
        assert_eq!(recovery["core_state"], "unavailable");
        assert_eq!(recovery["revision"], "candidate");
        assert_eq!(recovery["diagnostic"]["stage"], "validation");
        assert!(agent.application.applied_inputs.is_none());
        agent.publish_runtime(None).await;
        assert!(
            agent.local.status.lock().await["error"]
                .as_str()
                .is_some_and(|error| error.contains("validation")),
            "health ticks must preserve visible failure diagnostics"
        );
        // Starting a previously saved stopped configuration can fail after the
        // candidate branch was skipped. Final reporting must still be failed.
        agent.control = control::Durable {
            stopped: true,
            ..Default::default()
        };
        agent.application = Default::default();
        let stopped_key = agent
            .input_key("candidate", &camofy::digest(content))
            .await
            .unwrap();
        agent.application.succeeded(stopped_key, "candidate");
        desired.lock().await["command"] =
            json!({"id":"start-fails","type":"start","expires_at":now()+300});
        agent.sync().await.unwrap();
        let failed_start = reports.lock().await.last().unwrap().clone();
        assert_eq!(failed_start["status"], "failed");
        assert_eq!(failed_start["core_state"], "unavailable");
        assert!(agent.control.command_error.is_some());
        // A saved stopped artifact with a new identity-only revision needs no
        // download, and must replace old attempted identity/diagnostics.
        agent.control = control::Durable {
            stopped: true,
            ..Default::default()
        };
        agent.application = Default::default();
        let stopped_key = agent
            .input_key("candidate", &camofy::digest(content))
            .await
            .unwrap();
        agent.application.succeeded(stopped_key, "candidate");
        agent.application.diagnostic = Some(application::Diagnostic::new(
            "validation",
            "out_of_memory",
            "old failure",
        ));
        desired.lock().await["command"] = Value::Null;
        desired.lock().await["revision"] = json!("identity-only-revision");
        let count_before = downloads.load(Ordering::SeqCst);
        agent.sync().await.unwrap();
        let same_content = reports.lock().await.last().unwrap().clone();
        assert_eq!(same_content["status"], "applied");
        assert_eq!(same_content["attempted_revision"], "identity-only-revision");
        assert!(same_content["diagnostic"].is_null());
        assert!(same_content["retry"].is_null());
        assert_eq!(downloads.load(Ordering::SeqCst), count_before);
        server.abort();
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
    #[tokio::test]
    #[ignore = "requires CAMOFY_TEST_CORE built from cargo build --example mock-core"]
    async fn apply_reject_rollback_and_offline_restore() {
        let root = std::env::temp_dir().join(format!("camofy-agent-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let overlay = root.join("local.yaml");
        tokio::fs::write(&overlay, "mixed-port: 12345")
            .await
            .unwrap();
        let s = Settings {
            subscription_url: format!("http://127.0.0.1:1/sub/{}/router", "a".repeat(64)),
            cloud_url: "http://127.0.0.1:1".into(),
            token: "unused-test-credential".into(),
            mihomo: PathBuf::from(std::env::var("CAMOFY_TEST_CORE").unwrap()),
            data_dir: root.clone(),
            local_overlay: Some(overlay),
            controller_port: port,
            dns_redirect: false,
            web_listen: None,
        };
        let mut agent = Agent {
            s: s.clone(),
            http: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            secret: uuid::Uuid::new_v4().to_string(),
            child: None,
            cached: None,
            command: None,
            control: Default::default(),
            local: control::channel().0,
            proxies: Default::default(),
            application: Default::default(),
        };
        let content = "mixed-port: 7897\nproxies: [{name: mine, type: ss, server: example.com, port: 443}, {name: mine2, type: ss, server: example.com, port: 443}]\nproxy-groups: [{name: pick, type: select, proxies: [mine, mine2, DIRECT]}]\nrules: ['MATCH,pick']\n";
        let cache = Cached {
            revision: "first".into(),
            hash: camofy::digest(content),
            content: content.into(),
            selections: json!({"pick":"mine"}),
        };
        agent.apply(cache.clone()).await.unwrap();
        agent.proxies.selections = [("pick".into(), "mine".into())].into();
        agent.reconcile_proxies().await;
        assert_eq!(
            agent.local.status.lock().await["proxy_state"]["status"],
            "applied"
        );
        let pid_before = agent.child.as_ref().unwrap().id();
        assert!(
            agent
                .local_proxy("proxies.select", json!({"group":"pick","node":"mine2"}))
                .await
                .is_err()
        );
        agent
            .proxies
            .selections
            .insert("pick".into(), "mine2".into());
        agent.reconcile_proxies().await;
        assert_eq!(
            agent.child.as_ref().unwrap().id(),
            pid_before,
            "selection must not restart core"
        );
        assert_eq!(
            agent.local.status.lock().await["proxy_state"]["groups"][0]["now"],
            "mine2"
        );
        assert!(
            agent
                .local_proxy("proxies.select", json!({"group":"pick","node":"missing"}))
                .await
                .is_err()
        );
        agent
            .proxies
            .selections
            .insert("pick".into(), "missing".into());
        agent.reconcile_proxies().await;
        assert_eq!(
            agent.local.status.lock().await["proxy_state"]["errors"]["pick"],
            "node_missing"
        );
        agent.proxies = Default::default();
        let running = tokio::fs::read_to_string(root.join("mock-active.yaml"))
            .await
            .unwrap();
        assert!(running.contains("mixed-port: 12345"));
        assert!(running.contains("127.0.0.1"));
        assert_eq!(agent.test_delays().await.unwrap()["mine"], 42);
        let mut invalid = cache.clone();
        invalid.hash = "wrong".into();
        assert!(agent.apply(invalid).await.is_err());
        assert_eq!(agent.cached.as_ref().unwrap().revision, "first");
        for marker in ["fixture-reject: true", "fixture-fail-reload: true"] {
            let content = format!("{content}\n{marker}\n");
            let rejected = Cached {
                revision: "second".into(),
                hash: camofy::digest(&content),
                content,
                selections: json!({}),
            };
            assert!(agent.apply(rejected).await.is_err());
            assert_eq!(agent.cached.as_ref().unwrap().revision, "first");
            assert_eq!(
                tokio::fs::read_to_string(root.join("mock-active.yaml"))
                    .await
                    .unwrap(),
                running
            );
        }
        agent.child.take().unwrap().kill().await.unwrap();
        let restored: Cached =
            serde_json::from_slice(&tokio::fs::read(root.join("last-good.json")).await.unwrap())
                .unwrap();
        agent.secret = uuid::Uuid::new_v4().to_string();
        agent.cached = None;
        agent.apply(restored).await.unwrap(); // No request to unreachable cloud needed.
        assert_eq!(agent.cached.as_ref().unwrap().revision, "first");
        agent
            .control_core("stop", Some("stop-command"))
            .await
            .unwrap();
        assert!(agent.child.is_none());
        agent.apply(cache).await.unwrap();
        assert!(
            agent.child.is_none(),
            "sync cannot restart a deliberately stopped core"
        );
        let persisted: control::Durable =
            serde_json::from_slice(&tokio::fs::read(root.join("control.json")).await.unwrap())
                .unwrap();
        assert!(persisted.stopped);
        assert_eq!(persisted.command_id.as_deref(), Some("stop-command"));
        agent.control_core("start", None).await.unwrap();
        let pid = agent.child.as_ref().unwrap().id();
        agent.control_core("restart", None).await.unwrap();
        assert_ne!(pid, agent.child.as_ref().unwrap().id());
        agent.child.take().unwrap().kill().await.unwrap();
        // Only this test's freshly-created unique directory is removed.
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
