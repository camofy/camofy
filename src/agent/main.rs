//! Lightweight configuration consumer with a local, first-time authorization UI.
mod control;
mod pairing;
mod proxies;
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
        let state = if self.control.stopped {
            "stopped"
        } else if self.child.is_some() {
            "running"
        } else {
            "unavailable"
        };
        let mut status = self.local.status.lock().await;
        status["core_state"] = json!(state);
        status["revision"] = json!(self.cached.as_ref().map(|c| &c.revision));
        status["error"] = json!(error.or_else(|| self.control.command_error.clone()));
    }
    async fn halt(&mut self) -> Result<()> {
        // Remove only the Agent-owned redirect; never flush unrelated firewall state.
        if self.s.dns_redirect
            && let Ok(runtime) =
                tokio::fs::read_to_string(self.s.data_dir.join("running.yaml")).await
        {
            dns_redirect(false, &runtime).await?;
        }
        if let Some(mut child) = self.child.take() {
            child.kill().await?;
            child.wait().await?;
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
        result
    }
    fn api(&self, path: &str) -> String {
        format!("{}{}", self.s.cloud_url.trim_end_matches('/'), path)
    }
    fn core(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.s.controller_port)
    }
    async fn report(&self, revision: Option<&str>, status: &str, message: &str, extra: Value) {
        let mut body = json!({"revision":revision,"status":status,"message":message});
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
            self.http
                .put(self.core("/configs"))
                .bearer_auth(&self.secret)
                .query(&[("force", "true")])
                .json(&json!({"path":path}))
                .send()
                .await?
                .error_for_status()?;
        } else {
            self.child = Some(
                Command::new(&self.s.mihomo)
                    .arg("-d")
                    .arg(&self.s.data_dir)
                    .arg("-f")
                    .arg(path)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn()?,
            );
        }
        for _ in 0..30 {
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
        anyhow::bail!("Mihomo did not become healthy")
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
    async fn apply(&mut self, cache: Cached) -> Result<()> {
        ensure!(
            camofy::digest(&cache.content) == cache.hash,
            "artifact hash mismatch"
        );
        let runtime = self.runtime(&cache.content).await?;
        let candidate = self.s.data_dir.join("candidate.yaml");
        atomic(&candidate, runtime.as_bytes()).await?;
        let status = tokio::time::timeout(
            Duration::from_secs(30),
            Command::new(&self.s.mihomo)
                .arg("-t")
                .arg("-d")
                .arg(&self.s.data_dir)
                .arg("-f")
                .arg(&candidate)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .status(),
        )
        .await??;
        ensure!(status.success(), "Mihomo rejected candidate configuration");
        let running = self.s.data_dir.join("running.yaml");
        let previous = tokio::fs::read(&running).await.ok();
        atomic(&running, runtime.as_bytes()).await?;
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
                dns_redirect(true, &runtime).await?;
            }
            atomic(
                &self.s.data_dir.join("last-good.json"),
                &serde_json::to_vec(&cache)?,
            )
            .await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(e) = result {
            if let Some(old) = previous {
                atomic(&running, &old).await?;
                if !self.control.stopped && self.load_core(&running).await.is_err() {
                    if let Some(mut child) = self.child.take() {
                        let _ = child.kill().await;
                    }
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
            } else if let Some(mut child) = self.child.take() {
                let _ = child.kill().await;
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
            self.apply(cache).await?;
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
        if self
            .cached
            .as_ref()
            .is_none_or(|c| c.hash != hash.as_str().unwrap_or(""))
        {
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
                hash: hash.as_str().context("missing hash")?.into(),
                content: String::from_utf8(bytes)?,
                selections: desired["selections"].clone(),
            };
            if let Err(e) = self.apply(cache).await {
                self.report(
                    Some(revision),
                    "failed",
                    "candidate failed validation/application; inspect the agent locally",
                    json!({}),
                )
                .await;
                return Err(e);
            }
        }
        if let Some(cache) = &mut self.cached {
            cache.revision = revision.into();
            cache.selections = desired["selections"].clone();
            atomic(
                &self.s.data_dir.join("last-good.json"),
                &serde_json::to_vec(cache)?,
            )
            .await?;
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
                cached: None,
                command: None,
                control: self.control.clone(),
                local: self.local.clone(),
                proxies: self.proxies.clone(),
            };
            let id = id.to_string();
            // A slow node must not block realtime config application or the watchdog.
            tokio::spawn(async move {
                if let Ok(delays) = worker.test_delays().await {
                    worker
                        .report(
                            None,
                            "online",
                            "device delay test completed",
                            json!({"command_id":id,"delays":delays}),
                        )
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

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
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
                _ = shutdown() => return Ok(()),
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
    };
    if let Ok(bytes) = tokio::fs::read(s.data_dir.join("last-good.json")).await {
        let cache: Cached = serde_json::from_slice(&bytes)?;
        agent
            .apply(cache)
            .await
            .context("cannot restore last good configuration")?;
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
            _=poll.tick()=>{if agent.sync().await.is_err(){tracing::warn!("sync failed; keeping last good configuration");}},
            _=rx.changed()=>{if agent.sync().await.is_err(){tracing::warn!("sync failed; five-minute fallback remains active");}},
            _=health.tick()=>{
                agent.flush_results().await;
                agent.reconcile_proxies().await;
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
            _=shutdown()=>break,
        }
    }
    notification.abort();
    if let Some(c) = &agent.cached
        && agent.s.dns_redirect
    {
        let _ = dns_redirect(false, &c.content).await;
    }
    if let Some(mut child) = agent.child.take() {
        let _ = child.kill().await;
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
        agent
            .local_proxy("proxies.select", json!({"group":"pick","node":"mine2"}))
            .await
            .unwrap();
        assert_eq!(
            agent.child.as_ref().unwrap().id(),
            pid_before,
            "selection must not restart core"
        );
        assert_eq!(
            agent.local.status.lock().await["proxy_state"]["groups"][0]["now"],
            "mine2"
        );
        let local_state = proxies::Durable::load(&root).await.unwrap();
        assert_eq!(
            local_state.pending["pick"].as_deref(),
            Some("mine2"),
            "offline choice is durable"
        );
        assert!(
            agent
                .local_proxy("proxies.select", json!({"group":"pick","node":"missing"}))
                .await
                .is_err()
        );
        agent
            .local_proxy("proxies.select", json!({"group":"pick","node":null}))
            .await
            .unwrap();
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
