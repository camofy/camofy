//! WestData panel conversation through the Zyte API.
//!
//! Cloudflare gates this panel by the reputation of the source address: the platform egress is a
//! shared pool of short-lived addresses and is challenged constantly, and a challenged address is
//! answered with an interactive challenge that no plain HTTP client can clear. Zyte runs the panel
//! conversation in its own browser, on its own address, so the challenge never appears.
//!
//! Three properties of Zyte shape this module, all measured against the real panel:
//!
//! * The captcha image has to be fetched **inside** the browser session. Fetching it with Zyte's
//!   HTTP mode binds it to a different request chain, and every code is then rejected.
//! * The login POST has to be a browser request: Zyte's HTTP mode classifies this panel's POST as a
//!   ban response. It is issued from a neutral page, because `/` and `/index.php` redirect to the
//!   client area and that redirect regenerates the captcha we just solved.
//! * One session carries **one** login round. A second captcha round inside the same session is
//!   answered with `422 Session has expired`, so a rejected code is retried from a fresh session.
//!
//! Enabled by `CAMOFY_ZYTE_API_KEY`; without it the panel is fetched through the platform egress as
//! before. Subscription delivery never changes: the panel's own link and the conversion host are
//! still fetched through the platform egress.

use crate::captcha;
use crate::westdata::{self, Config, Failure, Hosts, Resolved, Service};
use crate::{retry, security};
use anyhow::{Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};
use std::time::Duration;

const API: &str = "https://api.zyte.com/v1/extract";
const RESPONSE_LIMIT: usize = 4 * 1024 * 1024;
/// One conversation: page, captcha, login, product page, activation. A cold Zyte session spends
/// most of its first browser request on browser start-up, so this is generous.
const CONVERSATION: Duration = Duration::from_secs(70);
/// A rejected code is cheapest to retry in a fresh session, and a misread is expected.
const LOGIN_ATTEMPTS: usize = 3;
/// Public page: no captcha image, and it does not redirect into the client area.
const NEUTRAL: &str = "cart.php";

/// A panel conversation failure that is safe to show in refresh history.
fn failure(step: &'static str, message: &'static str) -> anyhow::Error {
    anyhow::Error::new(Failure { step, message })
}

/// Panel transport failures are worth another refresh: each one starts a new Zyte session, which
/// means a new browser and a new address.
fn retryable(error: anyhow::Error) -> anyhow::Error {
    error.context(retry::SafeFailure {
        delay: Some(Duration::ZERO),
    })
}

/// This conversation cannot continue on its current session: either the panel rejected the code, or
/// Zyte dropped the session. Both are retried from a clean session, which is what the panel expects
/// to see anyway.
#[derive(Debug)]
struct Restart;

impl std::fmt::Display for Restart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("panel login needs a fresh browser session")
    }
}
impl std::error::Error for Restart {}

pub struct Zyte {
    api: url::Url,
    key: String,
    private: bool,
    hosts: Hosts,
    /// One session per conversation: Zyte keeps one address, one network stack and one cookie jar
    /// for its lifetime, and the panel must never see the conversation split across addresses.
    session: String,
}

impl Zyte {
    /// True when a Zyte key is configured, without building a session.
    pub fn configured() -> bool {
        std::env::var("CAMOFY_ZYTE_API_KEY").is_ok_and(|key| !key.trim().is_empty())
    }

    /// `None` means "not configured": the panel then uses the platform egress directly.
    pub fn from_env(private: bool) -> Result<Option<Self>> {
        let Ok(key) = std::env::var("CAMOFY_ZYTE_API_KEY") else {
            return Ok(None);
        };
        let key = key.trim().to_string();
        if key.is_empty() {
            return Ok(None);
        }
        let api = std::env::var("CAMOFY_ZYTE_API_URL").unwrap_or_else(|_| API.to_string());
        Ok(Some(Self {
            api: url::Url::parse(api.trim())?,
            key,
            private,
            hosts: Hosts::from_env(),
            session: uuid::Uuid::new_v4().to_string(),
        }))
    }

    /// Bounded JSON POST to the Zyte endpoint, always carrying this conversation's session.
    async fn extract(&self, mut payload: Map<String, Value>) -> Result<Value> {
        payload.insert("session".into(), json!({ "id": self.session }));
        let body = Value::Object(payload);
        let response = match security::post_json_basic(
            &self.api,
            &self.key,
            "",
            &body,
            RESPONSE_LIMIT,
            self.private,
        )
        .await
        {
            Ok(response) => response,
            // Zyte drops a session when its backend state is gone; a new session has to start.
            Err(error) if expired(&error) => bail!(retryable(anyhow::Error::new(Restart))),
            Err(error) => return Err(retryable(error)),
        };
        // Zyte also reports request problems in the body with a 2xx status.
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            if session_gone(None, error) {
                bail!(retryable(anyhow::Error::new(Restart)));
            }
            bail!(retryable(anyhow::anyhow!("Zyte API error: {error}")));
        }
        Ok(response)
    }

    /// A browser request: Zyte renders the page in its own browser and returns the DOM.
    async fn browser(&self, url: &str, actions: Option<Value>) -> Result<String> {
        let started = std::time::Instant::now();
        let mut payload = Map::new();
        payload.insert("url".into(), json!(url));
        payload.insert("browserHtml".into(), json!(true));
        if let Some(actions) = actions {
            payload.insert("actions".into(), actions);
        }
        let response = self.extract(payload).await?;
        let status = response
            .get("statusCode")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let html = response
            .get("browserHtml")
            .and_then(Value::as_str)
            .unwrap_or_default();
        ensure!(
            status == 200 && !html.is_empty(),
            retryable(anyhow::anyhow!("Zyte browser request failed")),
        );
        tracing::debug!(
            "zyte browser request finished in {} ms",
            started.elapsed().as_millis()
        );
        Ok(html.to_string())
    }

    /// A page read. HTTP mode is cheaper and faster and the panel never challenges a GET from
    /// Zyte's address, so it is tried first; a page that comes back empty or challenged is retried
    /// in the browser.
    async fn page(&self, url: &str) -> Result<String> {
        if let Ok(html) = self.http_page(url).await
            && !html.is_empty()
            && !westdata::cloudflare_wall(&html)
        {
            return Ok(html);
        }
        self.browser(url, None).await
    }

    /// Reads a page with Zyte's HTTP mode inside this session.
    async fn http_page(&self, url: &str) -> Result<String> {
        let mut payload = Map::new();
        payload.insert("url".into(), json!(url));
        payload.insert("httpResponseBody".into(), json!(true));
        let response = self.extract(payload).await?;
        let status = response
            .get("statusCode")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let encoded = response
            .get("httpResponseBody")
            .and_then(Value::as_str)
            .unwrap_or_default();
        ensure!(
            status == 200 && !encoded.is_empty(),
            retryable(anyhow::anyhow!("Zyte HTTP request failed")),
        );
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| retryable(anyhow::anyhow!("Zyte response is not base64")))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// One login round: fetch the captcha inside the session, read it, submit it from a page that
    /// has no captcha of its own. A rejected code restarts the conversation, because a second round
    /// inside this session is answered with an expired session.
    async fn login(&self, vision: &captcha::Vision, cfg: &Config) -> Result<()> {
        let area = self.hosts.client_area();
        let page = self.browser(&area, Some(grab_actions())).await?;
        if westdata::cloudflare_wall(&page) {
            bail!(failure(
                "westdata_egress",
                "面板出口被 Cloudflare 挑战或拦截，请稍后重试；已有配置不受影响。"
            ));
        }
        if westdata::logged_in(&page) {
            return Ok(());
        }
        let data = hidden_json(&page, "camofy-data")
            .ok_or_else(|| failure("westdata_login", "面板登录页没有提供验证码，请稍后重试。"))?;
        let token = data
            .get("token")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let image = captcha_image(&data)?;
        let code = captcha::read(vision, &image, self.private)
            .await
            .map_err(|e| e.context(failure("westdata_login", "验证码识别失败，请稍后重试。")))?;
        let neutral = format!("{}/{NEUTRAL}", self.hosts.site);
        let result = self
            .browser(
                &neutral,
                Some(login_actions(&self.hosts.site, token, &code, cfg)?),
            )
            .await?;
        let outcome = hidden_json(&result, "camofy-result")
            .ok_or_else(|| failure("westdata_login", "面板登录未返回结果，请稍后重试。"))?;
        if outcome.get("loggedIn").and_then(Value::as_bool) == Some(true) {
            return Ok(());
        }
        tracing::warn!("westdata login attempt was rejected");
        bail!(retryable(anyhow::Error::new(Restart)))
    }

    async fn subscription(&self, product: &str) -> Result<String> {
        let page = self.page(&self.hosts.product_page(product)).await?;
        if westdata::cloudflare_wall(&page) {
            bail!(failure(
                "westdata_egress",
                "面板出口被 Cloudflare 挑战或拦截，请稍后重试；已有配置不受影响。"
            ));
        }
        if !westdata::logged_in(&page) {
            bail!(failure(
                "westdata_login",
                "WestData 登录状态未被面板接受，请稍后重试。"
            ));
        }
        westdata::subscription_link(&page).ok_or_else(|| {
            failure(
                "westdata_subscription",
                "产品页没有提供订阅地址，请检查产品 ID 或账号状态。",
            )
        })
    }

    async fn activate(&self, product: &str) -> Result<()> {
        let page = self.page(&self.hosts.activate(product)).await?;
        if westdata::cloudflare_wall(&page) {
            bail!(failure(
                "westdata_egress",
                "面板出口被 Cloudflare 挑战或拦截，请稍后重试；已有配置不受影响。"
            ));
        }
        let text = visible_text(&page);
        if text != "success" {
            bail!(failure(
                "westdata_activate",
                "订阅更新开关打开失败，已保留上次成功配置。"
            ));
        }
        Ok(())
    }

    async fn services(&self) -> Result<Vec<Service>> {
        let url = format!("{}/clientarea.php?action=services", self.hosts.site);
        let page = self.page(&url).await?;
        if !westdata::logged_in(&page) {
            bail!(failure(
                "westdata_login",
                "WestData 登录状态未被面板接受，请稍后重试。"
            ));
        }
        let services = westdata::service_list(&page);
        ensure!(
            !services.is_empty(),
            failure(
                "westdata_subscription",
                "面板没有返回可管理的产品，请检查账号或稍后重试。"
            )
        );
        Ok(services)
    }

    /// Picks the service to manage: the configured one, or the only one the account has.
    async fn product_id(&self, cfg: &Config) -> Result<String> {
        if !cfg.product_id.is_empty() {
            return Ok(cfg.product_id.clone());
        }
        match self.services().await?.as_slice() {
            [only] => Ok(only.id.clone()),
            [] => bail!(failure(
                "westdata_subscription",
                "面板账号下没有可管理的产品，请先在面板购买或检查账号。"
            )),
            _ => bail!(failure(
                "westdata_subscription",
                "面板账号下有多个产品，请在本订阅源里选择要使用的产品。"
            )),
        }
    }
}

/// Runs one panel conversation per attempt, each on a fresh Zyte session, and returns the result the
/// caller fetches inside the ten-minute activation window.
pub async fn resolve(vision: &captcha::Vision, private: bool, cfg: &Config) -> Result<Resolved> {
    conversations(private, |zyte| async move {
        zyte.login(vision, cfg).await?;
        let product_id = zyte.product_id(cfg).await?;
        let base = zyte.subscription(&product_id).await?;
        zyte.activate(&product_id).await?;
        Ok(Resolved {
            clash: westdata::clash_url(&zyte.hosts.convert, &base)?,
            base,
        })
    })
    .await
}

/// Lists the services a panel account can manage, for the editor's scan action.
pub async fn discover(
    vision: &captcha::Vision,
    private: bool,
    cfg: &Config,
) -> Result<Vec<Service>> {
    conversations(private, |zyte| async move {
        zyte.login(vision, cfg).await?;
        zyte.services().await
    })
    .await
}

/// Retries a conversation only when the session itself has to be replaced: a rejected captcha or an
/// expired session. Anything else (a timeout, a transport failure, a missing subscription) is
/// reported as it is instead of paying for another login.
async fn conversations<T, F, Fut>(private: bool, mut conversation: F) -> Result<T>
where
    F: FnMut(Zyte) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut restarts = 0;
    loop {
        let zyte = Zyte::from_env(private)?.ok_or_else(|| {
            failure(
                "westdata_login",
                "未配置 Zyte API key，无法登录 WestData 面板。",
            )
        })?;
        match tokio::time::timeout(CONVERSATION, conversation(zyte)).await {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(error)) if error.downcast_ref::<Restart>().is_some() => {
                restarts += 1;
                if restarts >= LOGIN_ATTEMPTS {
                    bail!(failure(
                        "westdata_login",
                        "WestData 验证码连续识别失败，请稍后重试。"
                    ));
                }
                continue;
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                bail!(failure(
                    "westdata_timeout",
                    "WestData 面板响应超时，已保留上次成功配置。"
                ))
            }
        }
    }
}

/// True when Zyte reports the session as gone, which needs a new session.
fn expired(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .and_then(|e| e.status())
            .is_some_and(|status| session_gone(Some(status.as_u16()), ""))
    })
}

/// Zyte answers `422 Session has expired` when the session is gone, and reports the same condition
/// in the body of a 2xx response.
fn session_gone(status: Option<u16>, message: &str) -> bool {
    status == Some(422) || message.to_ascii_lowercase().contains("session")
}

/// Reads the captcha image inside the browser session and publishes it for the recogniser.
///
/// The image must come from the same session as the upcoming POST, so it is fetched by the page
/// itself; an HTTP-mode fetch would bind it to another request chain and the panel would reject
/// every code.
fn grab_actions() -> Value {
    json!([{
        "action": "evaluate",
        "source": "const f = document.querySelector('#signinform');\
                   const t = f && f.querySelector('input[name=token]');\
                   const x = new XMLHttpRequest();\
                   x.open('GET', 'includes/verifyimage.php?r=' + Math.random().toString(36).slice(2), false);\
                   x.overrideMimeType('text/plain; charset=x-user-defined');\
                   x.send();\
                   const raw = x.responseText || '';\
                   let s = '';\
                   for (let i = 0; i < raw.length; i++) s += String.fromCharCode(raw.charCodeAt(i) & 0xff);\
                   const d = document.createElement('div');\
                   d.id = 'camofy-data';\
                   d.style.display = 'none';\
                   d.innerText = JSON.stringify({ token: t ? t.value : '', bytes: raw.length, image: btoa(s) });\
                   document.body.appendChild(d);\
                   'ok';"
    }])
}

/// Submits the login form from the page itself. A synchronous request keeps this a single action:
/// Zyte only returns the DOM after every action has finished.
fn login_actions(site: &str, token: &str, code: &str, cfg: &Config) -> Result<Value> {
    let dologin = format!("{site}/dologin.php");
    let source = format!(
        "const body = new URLSearchParams({{\
             token: {token}, username: {user}, password: {password}, code: {code}, rememberme: '1'\
         }}).toString();\
         const x = new XMLHttpRequest();\
         x.open('POST', {dologin}, false);\
         x.setRequestHeader('Content-Type', 'application/x-www-form-urlencoded');\
         x.send(body);\
         const html = x.responseText || '';\
         const d = document.createElement('div');\
         d.id = 'camofy-result';\
         d.style.display = 'none';\
         d.innerText = JSON.stringify({{ status: x.status, loggedIn: /logout\\.php/.test(html) && !/signinform/.test(html) }});\
         document.body.appendChild(d);\
         'ok';",
        token = json!(token),
        user = json!(cfg.username),
        password = json!(cfg.password),
        code = json!(code),
        dologin = json!(dologin),
    );
    Ok(json!([{ "action": "evaluate", "source": source }]))
}

/// Decodes the captcha image the page published, rejecting an empty or malformed one.
fn captcha_image(data: &Value) -> Result<Vec<u8>> {
    let encoded = data
        .get("image")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| failure("westdata_login", "面板验证码图片无法解码，请稍后重试。"))?;
    ensure!(
        !bytes.is_empty(),
        failure("westdata_login", "面板验证码图片为空，请稍后重试。")
    );
    Ok(bytes)
}

/// Reads a JSON payload a page action published into a hidden `<div>`.
fn hidden_json(html: &str, id: &str) -> Option<Value> {
    let marker = format!("id=\"{id}\"");
    let start = html.find(&marker)?;
    let body = &html[html[start..].find('>')? + start + 1..];
    let end = body.find("</div>")?;
    let raw = body[..end].trim();
    serde_json::from_str(&unescape(raw)).ok()
}

/// The payload is published through `innerText`, so markup characters arrive escaped.
fn unescape(raw: &str) -> String {
    raw.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Plain text of a rendered page. The activation endpoint answers with bare `success`, which a
/// browser request wraps in a document.
fn visible_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut depth = 0_usize;
    for ch in html.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            username: "user@example.com".into(),
            password: "p\"w'd\\x".into(),
            product_id: "140991".into(),
        }
    }

    #[test]
    fn hidden_payloads_are_read_back_with_escaping() {
        let html = r#"<html><body><div id="camofy-data" style="display: none;">{&quot;token&quot;:&quot;abc&quot;,&quot;bytes&quot;:12,&quot;image&quot;:&quot;aGk=&quot;}</div><div id="x">no</div></body></html>"#;
        let data = hidden_json(html, "camofy-data").unwrap();
        assert_eq!(data["token"], "abc");
        assert_eq!(data["bytes"], 12);
        assert_eq!(captcha_image(&data).unwrap(), b"hi");
        assert!(hidden_json(html, "missing").is_none());
        assert!(hidden_json("<div id=\"camofy-data\">not json</div>", "camofy-data").is_none());
    }

    #[test]
    fn credentials_are_quoted_into_the_page_source() {
        let actions = login_actions("https://wd-gold.net", "tok\"en", "ab'cd", &config()).unwrap();
        let source = actions[0]["source"].as_str().unwrap();
        // Credentials are JSON string literals: quotes, apostrophes and backslashes cannot escape.
        assert!(source.contains(r#"password: "p\"w'd\\x""#));
        assert!(source.contains(r#"code: "ab'cd""#));
        assert!(source.contains(r#"token: "tok\"en""#));
        assert!(source.contains("https://wd-gold.net/dologin.php"));
        assert!(source.contains("logout\\.php"));
    }

    #[test]
    fn login_is_submitted_from_a_page_without_a_captcha() {
        // `/` and `/index.php` redirect into the client area, whose captcha image would overwrite
        // the code we just solved.
        assert_eq!(NEUTRAL, "cart.php");
        assert!(!NEUTRAL.contains("clientarea"));
        assert!(!NEUTRAL.starts_with('/'));
    }

    #[test]
    fn a_missing_key_disables_the_zyte_path() {
        // No key in this process, so the panel keeps using the platform egress.
        if std::env::var("CAMOFY_ZYTE_API_KEY").is_err() {
            assert!(Zyte::from_env(false).unwrap().is_none());
        }
    }

    #[test]
    fn activation_text_is_read_from_a_rendered_document() {
        assert_eq!(visible_text("<html><body>success</body></html>"), "success");
        assert_eq!(visible_text("  success\n"), "success");
        assert_ne!(visible_text("<html><body>failure</body></html>"), "success");
    }

    #[test]
    fn an_expired_session_is_recognised() {
        // Zyte answers 422 for a session it no longer has, and reports the same in an error body.
        assert!(session_gone(Some(422), ""));
        assert!(session_gone(
            None,
            "Session has expired. Try creating a new session."
        ));
        assert!(!session_gone(Some(403), ""));
        assert!(!session_gone(Some(520), ""));
        assert!(!session_gone(
            None,
            "Request contains unrecognized property url"
        ));
        assert!(!expired(&anyhow::anyhow!("no status")));
    }
}
