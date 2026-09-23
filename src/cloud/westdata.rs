//! WestData (wd-gold.com / wd-gold.net) panel subscriptions.
//!
//! The panel keeps its subscription link disabled by default and serves it only for ten
//! minutes after 「打开订阅更新开关」. A refresh therefore has to log in, read the current
//! Clash address, activate it again and only then fetch.
//!
//! Panel traffic follows the configured platform egress unless Zyte is enabled. A failed proxy
//! never silently falls back to direct; direct access must be selected explicitly by an admin.

use crate::{captcha, security};
use anyhow::{Result, bail, ensure};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

/// Captcha misreads are expected; each retry fetches a fresh image.
const LOGIN_ATTEMPTS: usize = 3;
/// The whole panel conversation stays inside one refresh attempt.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(24);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const PAGE_LIMIT: usize = 512 * 1024;
const IMAGE_LIMIT: usize = 256 * 1024;
/// A truncated agent string is answered with HTTP 403 by Cloudflare's WAF.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/152.0.0.0 Safari/537.36";
const ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,\
                      image/webp,*/*;q=0.8";

/// A panel conversation failure that is safe to show in refresh history.
#[derive(Debug)]
pub struct Failure {
    pub step: &'static str,
    pub message: &'static str,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}
impl std::error::Error for Failure {}

fn failure(step: &'static str, message: &'static str) -> anyhow::Error {
    anyhow::Error::new(Failure { step, message })
}

/// A Cloudflare interstitial is a property of the egress address, not of the request, so the
/// next attempt (which extracts a fresh platform address) may pass. The retry hint must not
/// hide the step: `history::failure` reports the panel step, `retry::delay` reads the hint.
fn wall(message: &'static str) -> anyhow::Error {
    failure("westdata_egress", message).context(crate::retry::SafeFailure {
        delay: Some(Duration::ZERO),
    })
}

/// Best available explanation for a client-facing error: panel failures carry their own
/// fixed wording, and anything else stays generic instead of exposing a transport chain.
pub fn describe(error: &anyhow::Error) -> String {
    match error.downcast_ref::<Failure>() {
        Some(failure) => failure.message.to_string(),
        None => "无法登录面板，请检查账号、订阅出口与验证码模型配置。".to_string(),
    }
}

/// The panel could not be reached because the egress address is challenged or blocked.
const WALL: &str = "平台出口被 Cloudflare 挑战或拦截（该出口 IP 信誉不足），请更换订阅出口后重试；\
                    已有配置不受影响。";

/// Hosts are fixed; only a local test harness may redirect them.
#[derive(Clone)]
pub struct Hosts {
    pub site: String,
    pub convert: String,
}

impl Hosts {
    pub fn from_env() -> Self {
        let site = std::env::var("CAMOFY_WESTDATA_SITE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "https://wd-gold.net".into());
        let convert = std::env::var("CAMOFY_WESTDATA_CONVERT")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "https://api.wd-turbo.com".into());
        Self {
            site: site.trim_end_matches('/').to_string(),
            convert: convert.trim_end_matches('/').to_string(),
        }
    }
    pub(crate) fn client_area(&self) -> String {
        format!("{}/clientarea.php", self.site)
    }
    pub(crate) fn product_page(&self, product: &str) -> String {
        format!(
            "{}/clientarea.php?action=productdetails&id={product}",
            self.site
        )
    }
    pub(crate) fn activate(&self, product: &str) -> String {
        format!(
            "{}/clientarea.php?action=productdetails&id={product}\
             &fuqingsocksAction=ActivateSublink&Serviceid={product}",
            self.site
        )
    }
}

#[derive(Clone)]
pub struct Config {
    pub username: String,
    pub password: String,
    /// Empty means "whatever the account has"; the refresh discovers it.
    pub product_id: String,
}

/// One service row from the panel's product list.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Service {
    pub id: String,
    pub name: String,
    pub status: String,
    pub next_due: String,
}

/// A source is WestData-managed as soon as the account area carries credentials.
pub fn config(data: &Value) -> Option<Config> {
    let entry = data.get("westdata")?.as_object()?;
    let username = entry.get("username")?.as_str()?.trim();
    let password = entry.get("password")?.as_str()?;
    if username.is_empty() || password.is_empty() {
        return None;
    }
    Some(Config {
        username: username.to_string(),
        password: password.to_string(),
        // Empty means "whatever the account has"; the refresh discovers it.
        product_id: entry
            .get("product_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string(),
    })
}

pub fn is_managed(data: &Value) -> bool {
    data.get("westdata").is_some_and(|v| v.is_object())
}

/// Validates and canonicalises the account block. An empty password keeps the stored one,
/// matching how every other credential in this codebase is edited.
pub fn normalize(data: &mut Value, old: Option<&Value>) -> Result<()> {
    let submitted = data.get("westdata").cloned().unwrap_or(Value::Null);
    if submitted.is_null() {
        data.as_object_mut().unwrap().remove("westdata");
        return Ok(());
    }
    ensure!(submitted.is_object(), "WestData 账号信息必须是对象");
    let previous = old.map(|o| &o["westdata"]);
    let username = submitted["username"].as_str().unwrap_or("").trim();
    ensure!(
        !username.is_empty() && username.len() <= 254 && !username.chars().any(char::is_control),
        "请填写 WestData 登录账号（邮箱）"
    );
    // An edit that omits the service id keeps the stored one; empty means "discover it".
    let product_id = match submitted["product_id"].as_str().unwrap_or("").trim() {
        "" => previous
            .and_then(|p| p["product_id"].as_str())
            .unwrap_or("")
            .trim()
            .to_string(),
        value => value.to_string(),
    };
    ensure!(
        product_id.len() <= 20 && product_id.bytes().all(|b| b.is_ascii_digit()),
        "WestData 产品 ID 必须是数字（可用「扫描产品」自动获取）"
    );
    let password = match submitted["password"].as_str().unwrap_or("") {
        "" => previous
            .and_then(|p| p["password"].as_str())
            .unwrap_or("")
            .to_string(),
        value => value.to_string(),
    };
    ensure!(!password.is_empty(), "请填写 WestData 登录密码");
    ensure!(password.len() <= 256, "WestData 密码过长");

    let mut entry = serde_json::Map::new();
    entry.insert("username".into(), json!(username));
    entry.insert("password".into(), json!(password));
    entry.insert("product_id".into(), json!(product_id));
    // Worker-owned results survive a settings edit.
    for key in ["subscription_url", "last_activate"] {
        if let Some(value) = previous.and_then(|p| p.get(key)) {
            entry.insert(key.into(), value.clone());
        }
    }
    data["westdata"] = Value::Object(entry);
    Ok(())
}

/// Never returns the password to a client.
pub fn redact(data: &mut Value) {
    if let Some(entry) = data.get_mut("westdata").and_then(Value::as_object_mut) {
        entry.remove("password");
    }
}

pub struct Resolved {
    /// Token link exactly as the panel shows it.
    pub base: String,
    /// Clash conversion of the token link, which is what a refresh fetches.
    pub clash: String,
}

/// Login → read the current subscription address → activate it. The returned Clash URL is
/// served for the next ten minutes, so it is fetched immediately by the caller.
pub async fn resolve(
    vision: &captcha::Vision,
    egress: Option<&str>,
    private: bool,
    cfg: &Config,
) -> Result<Resolved> {
    // A configured Zyte key runs the panel conversation inside their browser instead: Cloudflare
    // answers the shared egress pool with a challenge no plain HTTP client can clear.
    if crate::zyte::Zyte::configured() {
        return crate::zyte::resolve(vision, private, cfg).await;
    }
    let mut session = open(egress, private).await?;
    let conversation = async {
        session.login(vision, cfg).await?;
        let product_id = match cfg.product_id.is_empty() {
            false => cfg.product_id.clone(),
            // No service chosen yet: accept the only one, never guess between several.
            true => match session.discover().await?.as_slice() {
                [only] => only.id.clone(),
                [] => bail!(failure(
                    "westdata_subscription",
                    "面板账号下没有可管理的产品，请先在面板购买或检查账号。"
                )),
                _ => bail!(failure(
                    "westdata_subscription",
                    "面板账号下有多个产品，请在本订阅源里选择要使用的产品。"
                )),
            },
        };
        let base = session.subscription(&product_id).await?;
        session.activate(&product_id).await?;
        Ok::<_, anyhow::Error>(Resolved {
            clash: clash_url(&session.hosts.convert, &base)?,
            base,
        })
    };
    tokio::time::timeout(RESOLVE_TIMEOUT, conversation)
        .await
        .map_err(|_| {
            failure(
                "westdata_timeout",
                "WestData 面板响应超时，已保留上次成功配置。",
            )
        })?
}

/// Lists the services a panel account can manage, for the editor's scan action.
pub async fn discover(
    vision: &captcha::Vision,
    egress: Option<&str>,
    private: bool,
    cfg: &Config,
) -> Result<Vec<Service>> {
    if crate::zyte::Zyte::configured() {
        return crate::zyte::discover(vision, private, cfg).await;
    }
    let mut session = open(egress, private).await?;
    let conversation = async {
        session.login(vision, cfg).await?;
        session.discover().await
    };
    tokio::time::timeout(RESOLVE_TIMEOUT, conversation)
        .await
        .map_err(|_| failure("westdata_timeout", "WestData 面板响应超时，请稍后重试。"))?
}

async fn open(egress: Option<&str>, private: bool) -> Result<Session> {
    let hosts = Hosts::from_env();
    let target = url::Url::parse(&hosts.client_area())?;
    let client = security::egress_client(&target, egress, private, REQUEST_TIMEOUT).await?;
    Ok(Session {
        client,
        cookies: HashMap::new(),
        hosts,
        private,
    })
}

struct Session {
    client: security::Egress,
    cookies: HashMap<String, String>,
    hosts: Hosts,
    private: bool,
}

impl Session {
    async fn login(&mut self, vision: &captcha::Vision, cfg: &Config) -> Result<()> {
        let area = self.hosts.client_area();
        let (status, page) = self.get(&area, None).await?;
        // A challenged or blocked address answers with a challenge page and a non-success
        // status, so the wall must be recognised before the status is reported.
        if cloudflare_wall(&page) {
            return Err(wall(WALL));
        }
        if logged_in(&page) {
            return Ok(());
        }
        ensure!(
            status.is_success(),
            "WestData 面板返回 HTTP {}",
            status.as_u16()
        );
        for attempt in 1..=LOGIN_ATTEMPTS {
            let token = form_token(&page).ok_or_else(|| {
                failure(
                    "westdata_login",
                    "WestData 登录表单缺少 token，请稍后重试。",
                )
            })?;
            let fresh = format!(
                "{}/includes/verifyimage.php?r={}",
                self.hosts.site,
                uuid::Uuid::new_v4().simple()
            );
            let (status, image) = self.bytes(&fresh, Some(&area)).await?;
            ensure!(
                status.is_success(),
                "WestData 验证码接口返回 HTTP {}",
                status.as_u16()
            );
            let code = captcha::read(vision, &image, self.private)
                .await
                .map_err(|e| {
                    e.context(failure(
                        "westdata_login",
                        "验证码识别失败，请检查验证码模型配置。",
                    ))
                })?;
            let _ = self
                .post_form(
                    &format!("{}/dologin.php", self.hosts.site),
                    &[
                        ("token", token.as_str()),
                        ("username", cfg.username.as_str()),
                        ("password", cfg.password.as_str()),
                        ("code", code.as_str()),
                        ("rememberme", "on"),
                    ],
                    &area,
                )
                .await?;
            let (status, page) = self.get(&area, None).await?;
            if cloudflare_wall(&page) {
                return Err(wall(WALL));
            }
            if status.is_success() && logged_in(&page) {
                return Ok(());
            }
            tracing::warn!("westdata login attempt {attempt} was rejected");
        }
        Err(failure(
            "westdata_login",
            "WestData 登录失败：账号、密码或验证码不正确。",
        ))
    }

    /// Product list of the signed-in account, for choosing which service to use.
    async fn discover(&mut self) -> Result<Vec<Service>> {
        let url = format!("{}/clientarea.php?action=services", self.hosts.site);
        let (status, page) = self.get(&url, None).await?;
        if cloudflare_wall(&page) {
            bail!(wall(WALL));
        }
        ensure!(
            status.is_success(),
            "WestData 产品列表返回 HTTP {}",
            status.as_u16()
        );
        if !logged_in(&page) {
            bail!(failure(
                "westdata_login",
                "WestData 登录状态未被面板接受，请稍后重试。"
            ));
        }
        let services = service_list(&page);
        ensure!(
            !services.is_empty(),
            failure(
                "westdata_subscription",
                "面板没有返回可管理的产品，请检查账号或稍后重试。"
            )
        );
        Ok(services)
    }

    async fn subscription(&mut self, product: &str) -> Result<String> {
        let page_url = self.hosts.product_page(product);
        let (status, page) = self.get(&page_url, None).await?;
        if cloudflare_wall(&page) {
            bail!(wall(WALL));
        }
        ensure!(
            status.is_success(),
            "WestData 产品页返回 HTTP {}",
            status.as_u16()
        );
        if !logged_in(&page) {
            bail!(failure(
                "westdata_login",
                "WestData 登录状态未被面板接受，请稍后重试。"
            ));
        }
        subscription_link(&page).ok_or_else(|| {
            failure(
                "westdata_subscription",
                "产品页没有提供订阅地址，请检查产品 ID 或账号状态。",
            )
        })
    }

    async fn activate(&mut self, product: &str) -> Result<()> {
        let referer = self.hosts.product_page(product);
        let (status, body) = self
            .get(&self.hosts.activate(product), Some(&referer))
            .await?;
        if cloudflare_wall(&body) {
            bail!(wall(WALL));
        }
        ensure!(
            status.is_success(),
            "WestData 激活接口返回 HTTP {}",
            status.as_u16()
        );
        if body.trim() != "success" {
            bail!(failure(
                "westdata_activate",
                "订阅更新开关打开失败，已保留上次成功配置。"
            ));
        }
        Ok(())
    }

    fn cookie_header(&self) -> Option<String> {
        (!self.cookies.is_empty()).then(|| {
            self.cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ")
        })
    }

    fn remember(&mut self, response: &reqwest::Response) {
        for value in response
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
        {
            let Ok(text) = value.to_str() else { continue };
            let Some((name, rest)) = text.split_once('=') else {
                continue;
            };
            let value = rest.split(';').next().unwrap_or("").to_string();
            self.cookies.insert(name.trim().to_string(), value);
        }
    }

    fn request(
        &self,
        method: reqwest::Method,
        url: &str,
        referer: Option<&str>,
    ) -> Result<reqwest::RequestBuilder> {
        let mut builder = self
            .client
            .client()
            .request(method, url::Url::parse(url)?)
            .header(reqwest::header::USER_AGENT, UA)
            .header(reqwest::header::ACCEPT, ACCEPT)
            .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.8");
        if let Some(referer) = referer {
            builder = builder
                .header(reqwest::header::REFERER, referer)
                .header(reqwest::header::ORIGIN, self.hosts.site.clone());
        }
        if let Some(cookies) = self.cookie_header() {
            builder = builder.header(reqwest::header::COOKIE, cookies);
        }
        Ok(builder)
    }

    async fn get(
        &mut self,
        url: &str,
        referer: Option<&str>,
    ) -> Result<(reqwest::StatusCode, String)> {
        let response = self
            .request(reqwest::Method::GET, url, referer)?
            .send()
            .await?;
        self.remember(&response);
        let status = response.status();
        let text = String::from_utf8_lossy(&body(response, PAGE_LIMIT).await?).into_owned();
        self.note("GET", status, Some(&text));
        Ok((status, text))
    }

    async fn bytes(
        &mut self,
        url: &str,
        referer: Option<&str>,
    ) -> Result<(reqwest::StatusCode, Vec<u8>)> {
        let response = self
            .request(reqwest::Method::GET, url, referer)?
            .send()
            .await?;
        self.remember(&response);
        let status = response.status();
        let bytes = body(response, IMAGE_LIMIT).await?;
        self.note("GET", status, None);
        Ok((status, bytes))
    }

    async fn post_form(
        &mut self,
        url: &str,
        form: &[(&str, &str)],
        referer: &str,
    ) -> Result<(reqwest::StatusCode, String)> {
        let response = self
            .request(reqwest::Method::POST, url, Some(referer))?
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header(reqwest::header::ORIGIN, self.hosts.site.clone())
            .body(encode_form(form))
            .send()
            .await?;
        self.remember(&response);
        let status = response.status();
        let text = String::from_utf8_lossy(&body(response, PAGE_LIMIT).await?).into_owned();
        self.note("POST", status, Some(&text));
        Ok((status, text))
    }

    /// Never logs URLs, cookies, form values or page bodies: only the shape of the response.
    /// A wall is logged at warning level so an unreachable panel is diagnosable from the
    /// service log alone, which a bare "HTTP 403" was not.
    fn note(&self, step: &str, status: reqwest::StatusCode, page: Option<&str>) {
        if page.is_some_and(cloudflare_wall) {
            tracing::warn!(
                step,
                status = status.as_u16(),
                "westdata panel answered a Cloudflare challenge or block page"
            );
            return;
        }
        tracing::debug!(
            step,
            status = status.as_u16(),
            bytes = page.map(str::len).unwrap_or_default(),
            login_form = page.is_some_and(login_page),
            session = page.is_some_and(logged_in),
            "westdata panel"
        );
    }
}

async fn body(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    ensure!(
        response.content_length().is_none_or(|n| n <= limit as u64),
        "WestData 面板响应过大"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(bytes.len() + chunk.len() <= limit, "WestData 面板响应过大");
        bytes.extend(chunk);
    }
    Ok(bytes)
}

fn encode_form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

const FORM: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

fn encode(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, FORM).to_string()
}

/// The panel renders the login form only for anonymous visitors.
pub fn login_page(html: &str) -> bool {
    html.contains("id=\"signinform\"")
}

/// The authenticated client area always offers the logout link; no other page counts as a
/// session, which keeps a Cloudflare interstitial from being mistaken for a login.
pub fn logged_in(html: &str) -> bool {
    !login_page(html) && html.contains("logout.php")
}

/// Cloudflare answers a rejected client with a WAF block page or a managed challenge.
/// Every ordinary page of this zone also embeds `__CF$cv$params` bot-management script, so
/// that string must never be treated as a wall.
pub fn cloudflare_wall(html: &str) -> bool {
    html.contains("you have been blocked")
        || html.contains("_cf_chl_opt")
        || html.contains("Just a moment...")
        || html.contains("Attention Required! | Cloudflare")
}

/// `<input type="hidden" name="token" value="…">` from the login form.
pub fn form_token(html: &str) -> Option<String> {
    let rest = &html[html.find("name=\"token\"")?..];
    let value = &rest[rest.find("value=\"")? + 7..];
    let end = value.find('"')?;
    (end > 0).then(|| value[..end].to_string())
}

/// Product rows from `clientarea.php?action=services`. The panel marks every service row
/// with `data-element-id` and `data-type="service"`; the row order is
/// icon, name, price, next due, status, manage button. One account may hold several
/// services with the same name, so status and due date are part of the result.
pub fn service_list(html: &str) -> Vec<Service> {
    let mut services = Vec::new();
    let mut cursor = 0;
    while let Some(found) = html[cursor..].find("data-element-id=\"") {
        let at = cursor + found;
        cursor = at + 1;
        let after = &html[at + "data-element-id=\"".len()..];
        let id: String = after.chars().take_while(char::is_ascii_digit).collect();
        if id.is_empty() {
            continue;
        }
        // Only service rows: `data-type="service"` follows the id in the same cell.
        if !after
            .get(..80)
            .unwrap_or(after)
            .contains("data-type=\"service\"")
        {
            continue;
        }
        if services.iter().any(|s: &Service| s.id == id) {
            continue;
        }
        let start = html[..at].rfind("<tr").unwrap_or(at);
        let end = html[at..]
            .find("</tr>")
            .map(|offset| at + offset)
            .unwrap_or(html.len());
        let row = &html[start..end];
        let cells = cells(row);
        let name = cells
            .get(1)
            .map(|c| text(c))
            .filter(|n| !n.is_empty())
            .or_else(|| first_tag(row, "strong").map(|v| text(&v)))
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        services.push(Service {
            id,
            name,
            next_due: cells.get(3).map(|c| first_word(c)).unwrap_or_default(),
            status: cells.get(4).map(|c| text(c)).unwrap_or_default(),
        });
    }
    services
}

/// The panel repeats a date inside a hidden span, so only its first token is meaningful.
fn first_word(html: &str) -> String {
    text(html)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string()
}

fn cells(row: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(found) = row[cursor..].find("<td") {
        let open = cursor + found;
        let Some(close_open) = row[open..].find('>') else {
            break;
        };
        let body = open + close_open + 1;
        let Some(close) = row[body..].find("</td>") else {
            break;
        };
        out.push(row[body..body + close].to_string());
        cursor = body + close;
    }
    out
}

fn first_tag(html: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start = html.find(&open)?;
    let body = start + html[start..].find('>')? + 1;
    let end = html[body..].find(&close)?;
    Some(html[body..body + end].to_string())
}

/// Cell text without markup or entities.
fn text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The panel's own token link, e.g. `https://wd-turbo.com/subscribe/<token>`.
pub fn subscription_link(html: &str) -> Option<String> {
    let at = html.find("/subscribe/")?;
    let start = html[..at].rfind("https://")?;
    let token: String = html[at + "/subscribe/".len()..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    (token.len() >= 8).then(|| format!("{}{token}", &html[start..at + "/subscribe/".len()]))
}

/// The Clash conversion the panel itself offers for this token.
pub fn clash_url(convert: &str, base: &str) -> Result<String> {
    let base = url::Url::parse(base)?;
    ensure!(
        base.scheme() == "https"
            && base.path().starts_with("/subscribe/")
            && base.query().is_none(),
        "订阅地址格式不正确"
    );
    Ok(format!(
        "{}/sub?target=clash&emoji=true&udp=true&scv=true&new_name=true\
         &filename=WestData.yaml&url={}",
        convert.trim_end_matches('/'),
        encode(base.as_str())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGIN: &str = r#"<form id="signinform" method="post" action="https://wd-gold.net/dologin.php">
        <input type="hidden" name="token" value="f5240612496fef9fbcde1b40acde72a921f3250e" />
        <input id="inputEmail" type="email" name="username"><input id="inputPassword" type="password" name="password">
        <input id="inputCaptcha" type="text" name="code"><button id="login">登录</button></form>"#;

    // Fixtures only: no real panel account, token or service id ever appears in this repository.
    const PRODUCT: &str = r#"<div class="sublink"><input value="https://wd-turbo.com/subscribe/sample-token-0123456789">
        <a href="clash://install-config?url=https%3A%2F%2Fapi.wd-turbo.com%2Fsub">导入</a>
        <h2>订阅更新开关已关闭</h2></div>"#;
    const TOKEN: &str = "https://wd-turbo.com/subscribe/sample-token-0123456789";

    #[test]
    fn login_form_and_subscription_are_read_without_a_browser() {
        assert!(login_page(LOGIN));
        assert!(!login_page(PRODUCT));
        assert_eq!(
            form_token(LOGIN).as_deref(),
            Some("f5240612496fef9fbcde1b40acde72a921f3250e")
        );
        assert_eq!(form_token(PRODUCT), None);
        assert_eq!(subscription_link(PRODUCT).as_deref(), Some(TOKEN));
        assert_eq!(subscription_link(LOGIN), None);
        assert!(cloudflare_wall("<h1>Sorry, you have been blocked</h1>"));
        assert!(cloudflare_wall("<title>Just a moment...</title>"));
        assert!(!cloudflare_wall(PRODUCT));
        // This zone embeds Cloudflare's bot-management script on every ordinary page.
        assert!(!cloudflare_wall(
            "<script>window.__CF$cv$params={r:'x',t:'y'}</script><div>sublink</div>"
        ));
    }

    #[test]
    fn clash_conversion_matches_the_panel_link() {
        let url = clash_url("https://api.wd-turbo.com", TOKEN).unwrap();
        assert_eq!(
            url,
            "https://api.wd-turbo.com/sub?target=clash&emoji=true&udp=true&scv=true&new_name=true\
             &filename=WestData.yaml&url=https%3A%2F%2Fwd-turbo.com%2Fsubscribe%2Fsample-token-0123456789"
        );
        for bad in [
            "http://wd-turbo.com/subscribe/tokenvalue",
            "https://wd-turbo.com/other/tokenvalue",
            "https://wd-turbo.com/subscribe/tokenvalue?x=1",
        ] {
            assert!(clash_url("https://api.wd-turbo.com", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn service_rows_are_read_with_status_and_due_date() {
        // Structure taken from the panel's own product list; identifiers and names are fixtures.
        const SERVICES: &str = r#"
        <table><tbody>
        <tr onclick="clickableSafeRedirect(event, 'clientarea.php?action=productdetails&amp;id=111111', false)">
          <td class="text-center" data-element-id="111111" data-type="service"><img src="/assets/img/ssl/ssl-inactive-domain.png" data-toggle="tooltip" title="服务非激活状态"></td>
          <td><strong>Plan Alpha</strong></td>
          <td class="text-center" data-order="20.00">￥20.00CNY<br />每月</td>
          <td class="text-center"><span class="hidden">2025-04-08</span>2025-04-08</td>
          <td class="text-center">已终止</td>
          <td class="text-center"><a href="clientarea.php?action=productdetails&amp;id=111111" class="btn btn-block btn-info">管理产品</a></td>
        </tr>
        <tr onclick="clickableSafeRedirect(event, 'clientarea.php?action=productdetails&amp;id=222222', false)">
          <td class="text-center" data-element-id="222222" data-type="service"></td>
          <td><strong>Plan Beta</strong></td>
          <td class="text-center" data-order="110.00">￥110.00CNY<br />半年</td>
          <td class="text-center"><span class="hidden">2026-10-08</span>2026-10-08</td>
          <td class="text-center">有效的</td>
          <td class="text-center"><a href="clientarea.php?action=productdetails&amp;id=222222" class="btn">管理产品</a></td>
        </tr>
        <tr><td data-element-id="333333" data-type="domain">domain.example</td><td>example.com</td></tr>
        </tbody></table>"#;
        let list = service_list(SERVICES);
        assert_eq!(list.len(), 2, "domains are not services: {list:?}");
        assert_eq!(
            (list[0].id.as_str(), list[0].name.as_str()),
            ("111111", "Plan Alpha")
        );
        assert_eq!(
            (list[0].status.as_str(), list[0].next_due.as_str()),
            ("已终止", "2025-04-08")
        );
        assert_eq!(
            (list[1].id.as_str(), list[1].name.as_str()),
            ("222222", "Plan Beta")
        );
        assert_eq!(
            (list[1].status.as_str(), list[1].next_due.as_str()),
            ("有效的", "2026-10-08")
        );
        assert!(service_list("<html><body>no services</body></html>").is_empty());
        assert!(service_list("").is_empty());
    }

    #[test]
    fn a_cloudflare_wall_is_reported_and_retried_on_a_fresh_address() {
        let error = wall(WALL);
        // The refresh history must name the step, not a transport chain.
        assert_eq!(
            crate::history::failure(&error, "westdata").0,
            "westdata_egress"
        );
        // The worker must retry: the next attempt extracts a different egress address.
        assert_eq!(crate::retry::delay(&error), Some(std::time::Duration::ZERO));
        assert!(describe(&error).contains("Cloudflare"));
        // A client-facing message never exposes the transport chain.
        assert!(!describe(&error).contains("provider request failed"));
        assert_eq!(
            crate::westdata::describe(&anyhow::anyhow!("connection reset by peer")),
            "无法登录面板，请检查账号、订阅出口与验证码模型配置。"
        );
    }

    #[test]
    fn form_encoding_is_strict() {
        assert_eq!(
            encode_form(&[
                ("username", "user@example.com"),
                ("password", "example-password-123"),
                ("code", "AB12"),
            ]),
            "username=user%40example.com&password=example-password-123&code=AB12"
        );
    }

    #[test]
    fn accounts_are_validated_and_never_returned() {
        let mut data = json!({"type":"source","name":"wd","url":"", "westdata":{
            "username":" user@example.com ", "password":"example-password-123", "product_id":"123456"}});
        normalize(&mut data, None).unwrap();
        assert_eq!(data["westdata"]["username"], "user@example.com");
        assert_eq!(data["westdata"]["product_id"], "123456");
        assert_eq!(
            config(&data).map(|c| (c.username, c.password, c.product_id)),
            Some((
                "user@example.com".into(),
                "example-password-123".into(),
                "123456".into()
            ))
        );

        // Editing without a password keeps the stored one.
        let old = data.clone();
        let mut edit =
            json!({"type":"source","name":"wd","westdata":{"username":"user@example.com"}});
        normalize(&mut edit, Some(&old)).unwrap();
        assert_eq!(edit["westdata"]["password"], "example-password-123");
        assert_eq!(edit["westdata"]["product_id"], "123456");

        // Clearing the block detaches the account.
        let mut cleared = json!({"type":"source","name":"wd","westdata":null});
        normalize(&mut cleared, Some(&old)).unwrap();
        assert!(cleared.get("westdata").is_none());
        assert!(!is_managed(&cleared));

        // An empty service id is allowed: the refresh discovers the account's only product.
        let mut auto = json!({"type":"source","name":"wd","westdata":{
            "username":"user@example.com","password":"example-password-123"}});
        normalize(&mut auto, None).unwrap();
        assert_eq!(auto["westdata"]["product_id"], "");
        assert_eq!(
            config(&auto).map(|c| c.product_id),
            Some(String::new()),
            "an account without a chosen product is still managed"
        );

        let mut redacted = data.clone();
        redact(&mut redacted);
        assert!(redacted["westdata"].get("password").is_none());
        assert!(!redacted.to_string().contains("example-password-123"));

        for bad in [
            json!({"westdata":{"username":"","password":"x","product_id":"1"}}),
            json!({"westdata":{"username":"a","password":"","product_id":"1"}}),
            json!({"westdata":{"username":"a","password":"x","product_id":"14a"}}),
            json!({"westdata":"nope"}),
        ] {
            assert!(normalize(&mut bad.clone(), None).is_err(), "{bad}");
        }
    }

    #[test]
    fn worker_owned_fields_survive_an_edit() {
        let old = json!({"westdata":{"username":"user@example.com","password":"p","product_id":"123456",
            "subscription_url":TOKEN,"last_activate":42}});
        let mut edit = json!({"westdata":{"username":"user@example.com"}});
        normalize(&mut edit, Some(&old)).unwrap();
        assert_eq!(
            edit["westdata"]["subscription_url"],
            old["westdata"]["subscription_url"]
        );
        assert_eq!(edit["westdata"]["last_activate"], 42);
    }

    /// Live panel check for a real account. Credentials and the vision key are read from the
    /// environment at run time; no account, token or key is ever stored in this repository.
    ///
    /// ```text
    /// CAMOFY_VISION_API_KEY=… CAMOFY_WESTDATA_USER=… CAMOFY_WESTDATA_PASS=… \
    /// CAMOFY_WESTDATA_PRODUCT=… cargo test --all-features --bin camofy-cloud -- \
    ///     --ignored --nocapture live_panel
    /// ```
    #[tokio::test]
    #[ignore = "live check against a real panel; needs account and vision environment variables"]
    async fn live_panel_reads_a_working_clash_subscription() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
        let required = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| panic!("{name} is required for the live check"))
        };
        let cfg = Config {
            username: required("CAMOFY_WESTDATA_USER"),
            password: required("CAMOFY_WESTDATA_PASS"),
            product_id: required("CAMOFY_WESTDATA_PRODUCT"),
        };
        let vision = captcha::Vision::from_env()
            .unwrap()
            .expect("CAMOFY_VISION_API_KEY is required for the live check");
        // A workstation behind a transparent fake-IP proxy needs private resolution;
        // production resolves public addresses only.
        let private = std::env::var("CAMOFY_WESTDATA_PRIVATE").as_deref() == Ok("1");

        let resolved = resolve(&vision, None, private, &cfg)
            .await
            .expect("panel login and subscription resolution");
        // Print shape only: the token itself is a live credential.
        println!(
            "resolved base: {}… ({} bytes), clash conversion {} bytes",
            &resolved.base[..resolved.base.len().min(34)],
            resolved.base.len(),
            resolved.clash.len()
        );

        let fetched = security::fetch(&resolved.clash, None, private)
            .await
            .expect("clash conversion must be served inside the activation window");
        assert!(
            camofy::engine::parse(&fetched.content).is_ok(),
            "panel conversion is not a Clash mapping"
        );
        let nodes = fetched.content.matches("- name:").count();
        println!("clash nodes: {nodes}; usage: {:?}", fetched.usage.status);
        assert!(nodes > 0, "panel conversion carried no proxies");

        let services = discover(&vision, None, private, &cfg)
            .await
            .expect("panel product list");
        assert!(!services.is_empty(), "panel listed no products");
        for service in &services {
            println!(
                "service {} · {} · {} · {}",
                service.id, service.name, service.status, service.next_due
            );
        }
        assert!(
            services.iter().any(|s| s.id == cfg.product_id),
            "the configured service must appear in the account's product list"
        );
    }
}
