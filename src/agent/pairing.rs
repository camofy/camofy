use super::Settings;
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Query, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, watch};

const CLOUD: &str = "https://camofy.app";
const CLIENT: &str = "camofy-agent";
#[derive(Clone)]
struct Web {
    key_hash: String,
    admins: Arc<Mutex<Vec<(String, Instant)>>>,
    login_attempt: Arc<Mutex<Instant>>,
    settings: PathBuf,
    http: reqwest::Client,
    inner: Arc<Mutex<Inner>>,
    tx: watch::Sender<Option<String>>,
    local: super::control::Local,
}
struct Inner {
    bound: bool,
    cloud: String,
    identity: String,
    flow: Option<Flow>,
    phase: String,
}
struct Flow {
    state: String,
    session: String,
    until: Instant,
}
type WebResult<T> = std::result::Result<T, (StatusCode, Json<Value>)>;
fn fail(status: StatusCode, message: &str) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"error":message})))
}
fn secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
fn opaque(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn session(h: &HeaderMap) -> Option<&str> {
    h.get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|v| v.trim().strip_prefix("camofy_pair="))
        .filter(|v| opaque(v))
}
fn local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_private() || v.is_loopback(),
        IpAddr::V6(v) => v.is_loopback() || (v.segments()[0] & 0xfe00) == 0xfc00,
    }
}
fn origin(h: &HeaderMap) -> Option<String> {
    let host = h.get(header::HOST)?.to_str().ok()?;
    let u = url::Url::parse(&format!("http://{host}")).ok()?;
    let local = match u.host() {
        Some(url::Host::Ipv4(v)) => local_ip(v.into()),
        Some(url::Host::Ipv6(v)) => local_ip(v.into()),
        Some(url::Host::Domain("localhost")) => true,
        _ => false,
    };
    (local
        && u.username().is_empty()
        && u.password().is_none()
        && u.path() == "/"
        && u.query().is_none()
        && u.fragment().is_none())
    .then(|| u.origin().ascii_serialization())
}
fn cloud_origin(value: &str) -> Result<String> {
    let u = url::Url::parse(value.trim())?;
    ensure!(
        u.scheme() == "https"
            || (u.scheme() == "http"
                && ["localhost", "127.0.0.1", "[::1]"].contains(&u.host_str().unwrap_or(""))),
        "cloud requires HTTPS"
    );
    ensure!(
        u.username().is_empty()
            && u.password().is_none()
            && u.path() == "/"
            && u.query().is_none()
            && u.fragment().is_none(),
        "cloud must be an origin"
    );
    Ok(u.origin().ascii_serialization())
}
async fn guard(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if !local_ip(peer.ip()) || origin(request.headers()).is_none() {
        return fail(StatusCode::FORBIDDEN, "请通过路由器的局域网 IP 地址访问。").into_response();
    }
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    h.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    h.insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    h.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    h.insert(header::CONTENT_SECURITY_POLICY,"default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'".parse().unwrap());
    response
}
pub(super) async fn start(
    s: &Settings,
    settings: PathBuf,
    local: super::control::Local,
) -> Result<watch::Receiver<Option<String>>> {
    let (tx, rx) = watch::channel(None);
    let Some(listen) = &s.web_listen else {
        ensure!(s.bound(), "unbound Agent needs web_listen");
        return Ok(rx);
    };
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .context("cannot bind local pairing UI")?;
    let identity = serde_json::from_slice::<Value>(&tokio::fs::read(&settings).await?)
        .ok()
        .and_then(|v| v["identity_name"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let web = Web {
        key_hash: {
            let path = s.data_dir.join("local-admin-key");
            let key = match tokio::fs::read_to_string(&path).await {
                Ok(k) => k,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let k = secret();
                    super::atomic(&path, k.as_bytes()).await?;
                    k
                }
                Err(e) => return Err(e.into()),
            };
            camofy::digest(key.trim())
        },
        admins: Arc::new(Mutex::new(Vec::new())),
        login_attempt: Arc::new(Mutex::new(Instant::now() - Duration::from_secs(2))),
        settings,
        http: reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(12))
            .build()?,
        inner: Arc::new(Mutex::new(Inner {
            bound: s.bound(),
            cloud: if s.cloud_url.is_empty() {
                CLOUD.into()
            } else {
                s.cloud_url.clone()
            },
            identity,
            flow: None,
            phase: "idle".into(),
        })),
        tx,
        local,
    };
    let router = Router::new()
        .route("/", get(root))
        .route("/bind", get(page))
        .route("/bind/complete", get(complete))
        .route("/pair.js", get(script))
        .route("/api/pair/status", get(status))
        .route("/api/pair/start", post(begin))
        .route("/api/core/control", post(control))
        .route("/api/local/login", post(login))
        .route("/api/proxies", get(proxy_status).post(proxy_action))
        .fallback(get(root))
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn(guard))
        .with_state(web);
    tracing::info!("local binding UI listening at {listen}");
    tokio::spawn(async move {
        if axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .is_err()
        {
            tracing::error!("local binding UI stopped");
        }
    });
    Ok(rx)
}
async fn root(State(w): State<Web>, h: HeaderMap) -> Response {
    if !w.inner.lock().await.bound {
        Redirect::temporary("/bind").into_response()
    } else {
        page(State(w), h).await
    }
}
async fn page(State(w): State<Web>, h: HeaderMap) -> Response {
    let cookie = session(&h).map(str::to_owned).unwrap_or_else(secret);
    let cloud = w
        .inner
        .lock()
        .await
        .cloud
        .clone()
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
    let html = include_str!("pair.html")
        .replace("__CSRF__", &cookie)
        .replace(
            "value=\"https://camofy.app/\"",
            &format!("value=\"{cloud}/\""),
        );
    (
        [(
            header::SET_COOKIE,
            format!("camofy_pair={cookie}; Path=/; HttpOnly; SameSite=Lax; Max-Age=900"),
        )],
        Html(html),
    )
        .into_response()
}
async fn script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("pair.js"),
    )
}
#[derive(Deserialize)]
struct Callback {
    state: String,
}
async fn complete(State(w): State<Web>, h: HeaderMap, Query(q): Query<Callback>) -> Response {
    let inner = w.inner.lock().await;
    let valid = inner.flow.as_ref().is_some_and(|f| {
        f.state == q.state && Some(f.session.as_str()) == session(&h) && Instant::now() < f.until
    });
    if !valid {
        return (StatusCode::BAD_REQUEST,Html("<meta charset=utf-8><p>此回跳已失效或不属于当前浏览器。请返回路由器绑定页。</p><a href=/bind>返回绑定页</a>")).into_response();
    }
    drop(inner);
    page(State(w), h).await
}
async fn status(State(w): State<Web>, h: HeaderMap) -> Json<Value> {
    let inner = w.inner.lock().await;
    let own = inner
        .flow
        .as_ref()
        .is_some_and(|f| Some(f.session.as_str()) == session(&h));
    let authorized = admin(&w, &h).await;
    let runtime = if authorized {
        w.local.status.lock().await.clone()
    } else {
        json!({"core_state":"locked"})
    };
    Json(
        json!({"authorized":authorized,"bound":inner.bound,"cloud_url":inner.cloud,"identity_name":runtime["identity_name"].as_str().unwrap_or(&inner.identity),"runtime":runtime,"phase":if own{inner.phase.as_str()}else{"idle"}}),
    )
}
#[derive(Deserialize)]
struct Action {
    action: String,
}
async fn control(
    State(w): State<Web>,
    h: HeaderMap,
    Json(a): Json<Action>,
) -> WebResult<StatusCode> {
    if !admin(&w, &h).await {
        return Err(fail(StatusCode::UNAUTHORIZED, "请先解锁本地控制台"));
    }
    let local = origin(&h).ok_or_else(|| fail(StatusCode::FORBIDDEN, "invalid local origin"))?;
    let cookie = session(&h).ok_or_else(|| fail(StatusCode::FORBIDDEN, "请刷新页面后重试"))?;
    if h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(local.as_str())
        || h.get("x-camofy-csrf").and_then(|v| v.to_str().ok()) != Some(cookie)
    {
        return Err(fail(StatusCode::FORBIDDEN, "invalid request origin"));
    }
    if !w.inner.lock().await.bound {
        return Err(fail(StatusCode::CONFLICT, "请先绑定云端身份"));
    }
    if !["start", "stop", "restart"].contains(&a.action.as_str()) {
        return Err(fail(StatusCode::BAD_REQUEST, "unknown action"));
    }
    w.local
        .tx
        .try_send(super::control::Request::Core(a.action))
        .map_err(|_| fail(StatusCode::CONFLICT, "操作正在处理中，请稍后重试"))?;
    Ok(StatusCode::ACCEPTED)
}
async fn admin(w: &Web, h: &HeaderMap) -> bool {
    let key = h
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            s.split(';')
                .find_map(|v| v.trim().strip_prefix("camofy_admin="))
        });
    let mut sessions = w.admins.lock().await;
    sessions.retain(|(_, until)| *until > Instant::now());
    key.is_some_and(|k| sessions.iter().any(|(saved, _)| saved == k))
}
fn csrf(h: &HeaderMap) -> WebResult<()> {
    let expected = origin(h).ok_or_else(|| fail(StatusCode::FORBIDDEN, "invalid origin"))?;
    if h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(&expected)
        || session(h).is_none()
        || h.get("x-camofy-csrf").and_then(|v| v.to_str().ok()) != session(h)
    {
        return Err(fail(StatusCode::FORBIDDEN, "请刷新页面后重试"));
    }
    Ok(())
}
async fn login(
    State(w): State<Web>,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> WebResult<impl IntoResponse> {
    csrf(&h)?;
    let mut last = w.login_attempt.lock().await;
    if last.elapsed() < Duration::from_secs(1) {
        return Err(fail(StatusCode::TOO_MANY_REQUESTS, "请稍后重试"));
    }
    *last = Instant::now();
    drop(last);
    if camofy::digest(body["key"].as_str().unwrap_or("").trim()) != w.key_hash {
        return Err(fail(StatusCode::UNAUTHORIZED, "管理密钥不正确"));
    }
    let key = secret();
    let mut sessions = w.admins.lock().await;
    sessions.retain(|(_, until)| *until > Instant::now());
    if sessions.len() >= 32 {
        sessions.remove(0);
    }
    sessions.push((key.clone(), Instant::now() + Duration::from_secs(43200)));
    Ok((
        [(
            header::SET_COOKIE,
            format!("camofy_admin={key}; Path=/; HttpOnly; SameSite=Strict; Max-Age=43200"),
        )],
        StatusCode::NO_CONTENT,
    ))
}
async fn proxy_status(State(w): State<Web>, h: HeaderMap) -> WebResult<Json<Value>> {
    if !admin(&w, &h).await {
        return Err(fail(StatusCode::UNAUTHORIZED, "请先解锁本地控制台"));
    }
    Ok(Json(w.local.status.lock().await["proxy_state"].clone()))
}
async fn proxy_action(
    State(w): State<Web>,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> WebResult<Json<Value>> {
    csrf(&h)?;
    if !admin(&w, &h).await {
        return Err(fail(StatusCode::UNAUTHORIZED, "请先解锁本地控制台"));
    }
    let method = body["method"].as_str().unwrap_or("");
    if !["proxies.list", "proxies.select", "proxies.delay"].contains(&method) {
        return Err(fail(StatusCode::BAD_REQUEST, "unsupported method"));
    }
    let (reply, rx) = tokio::sync::oneshot::channel();
    w.local
        .tx
        .try_send(super::control::Request::Proxy {
            method: method.into(),
            params: body["params"].clone(),
            reply,
        })
        .map_err(|_| fail(StatusCode::CONFLICT, "操作繁忙，请稍后重试"))?;
    match tokio::time::timeout(Duration::from_secs(30), rx).await {
        Ok(Ok(Ok(value))) => Ok(Json(value)),
        Ok(Ok(Err(e))) => Err(fail(StatusCode::BAD_REQUEST, &e)),
        _ => Err(fail(
            StatusCode::GATEWAY_TIMEOUT,
            "操作结果未确认，请刷新状态，不要重复提交",
        )),
    }
}
#[derive(Deserialize)]
struct Begin {
    cloud_url: Option<String>,
    device_name: String,
}
async fn begin(State(w): State<Web>, h: HeaderMap, Json(b): Json<Begin>) -> WebResult<Json<Value>> {
    let local = origin(&h).ok_or_else(|| fail(StatusCode::FORBIDDEN, "invalid local origin"))?;
    let cookie = session(&h).ok_or_else(|| fail(StatusCode::FORBIDDEN, "请重新打开绑定页。"))?;
    if h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(local.as_str())
        || h.get("x-camofy-csrf").and_then(|v| v.to_str().ok()) != Some(cookie)
    {
        return Err(fail(StatusCode::FORBIDDEN, "invalid request origin"));
    }
    let cloud = cloud_origin(b.cloud_url.as_deref().unwrap_or(CLOUD))
        .map_err(|_| fail(StatusCode::BAD_REQUEST, "服务器地址必须是 HTTPS 根地址。"))?;
    if b.device_name.trim().is_empty() || b.device_name.len() > 120 {
        return Err(fail(
            StatusCode::BAD_REQUEST,
            "请输入设备名称（不超过 120 字节）。",
        ));
    }
    let state = secret();
    {
        let mut inner = w.inner.lock().await;
        if inner.bound {
            return Err(fail(
                StatusCode::CONFLICT,
                "设备已绑定；请在云端管理其身份。",
            ));
        }
        if inner
            .flow
            .as_ref()
            .is_some_and(|f| Instant::now() < f.until)
            && ["starting", "pending"].contains(&inner.phase.as_str())
        {
            return Err(fail(
                StatusCode::CONFLICT,
                "已有授权正在进行，请完成授权或等待其过期。",
            ));
        }
        inner.flow = Some(Flow {
            state: state.clone(),
            session: cookie.into(),
            until: Instant::now() + Duration::from_secs(600),
        });
        inner.phase = "starting".into();
    }
    let result=async{
        let reply=w.http.post(format!("{cloud}/api/oauth/device_authorization")).form(&json!({"client_id":CLIENT,"scope":"device:sync","device_name":b.device_name.trim(),"return_uri":format!("{local}/bind/complete"),"state":state})).send().await?.error_for_status()?.json::<Value>().await?;
        let device=reply["device_code"].as_str().filter(|v|opaque(v)).context("invalid device code")?.to_string();
        let verification=reply["verification_uri_complete"].as_str().context("missing verification URI")?.to_string();
        let target=url::Url::parse(&verification)?;
        ensure!(target.origin().ascii_serialization()==cloud&&target.path()=="/authorize"&&target.username().is_empty()&&target.password().is_none()&&target.fragment().is_none(),"invalid authorization origin");
        let interval=reply["interval"].as_u64().unwrap_or(5).clamp(5,60);
        let expires=reply["expires_in"].as_u64().unwrap_or(600).clamp(30,600);
        {let mut i=w.inner.lock().await;i.cloud=cloud.clone();i.phase="pending".into();if let Some(f)=&mut i.flow{f.until=Instant::now()+Duration::from_secs(expires);}}
        tokio::spawn(poll(w.clone(),cloud,device,state,interval,expires));
        Ok::<_,anyhow::Error>(Json(json!({"verification_uri":verification,"user_code":reply["user_code"]})))
    }.await;
    match result {
        Ok(v) => Ok(v),
        Err(_) => {
            w.inner.lock().await.phase = "error".into();
            Err(fail(
                StatusCode::BAD_GATEWAY,
                "无法连接云端授权服务，请检查服务器地址与网络后重试。",
            ))
        }
    }
}
async fn poll(
    w: Web,
    cloud: String,
    device: String,
    state: String,
    mut interval: u64,
    expires: u64,
) {
    let deadline = Instant::now() + Duration::from_secs(expires);
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        {
            let i = w.inner.lock().await;
            if i.bound || i.flow.as_ref().is_none_or(|f| f.state != state) {
                return;
            }
        }
        let reply=w.http.post(format!("{cloud}/api/oauth/token")).form(&json!({"grant_type":"urn:ietf:params:oauth:grant-type:device_code","client_id":CLIENT,"device_code":device})).send().await;
        let Ok(reply) = reply else {
            continue;
        };
        let ok = reply.status().is_success();
        let Ok(value) = reply.json::<Value>().await else {
            continue;
        };
        if !ok {
            match value["error"].as_str().unwrap_or("") {
                "authorization_pending" => continue,
                "slow_down" => {
                    interval = (interval + 5).min(600);
                    continue;
                }
                "access_denied" => {
                    set_phase(&w, &state, "denied").await;
                    return;
                }
                "expired_token" => break,
                _ => {
                    set_phase(&w, &state, "error").await;
                    return;
                }
            }
        }
        // Serialize the final write against another browser starting a replacement flow.
        let mut i = w.inner.lock().await;
        if i.bound
            || !i
                .flow
                .as_ref()
                .is_some_and(|f| f.state == state && Instant::now() < f.until)
        {
            return;
        }
        match save_binding(&w, &cloud, &value).await {
            Ok(subscription) => {
                if i.flow.as_ref().is_some_and(|f| f.state == state) {
                    i.bound = true;
                    i.phase = "bound".into();
                    i.identity = value["identity_name"].as_str().unwrap_or("").into();
                    w.tx.send_replace(Some(subscription));
                }
                return;
            }
            Err(_) => {
                i.phase = "save_error".into();
                return;
            }
        }
    }
    set_phase(&w, &state, "expired").await;
}
async fn set_phase(w: &Web, state: &str, phase: &str) {
    let mut i = w.inner.lock().await;
    if i.flow.as_ref().is_some_and(|f| f.state == state) {
        i.phase = phase.into();
    }
}
async fn save_binding(w: &Web, cloud: &str, value: &Value) -> Result<String> {
    let token = value["access_token"]
        .as_str()
        .filter(|v| opaque(v))
        .context("invalid token")?;
    ensure!(
        value["token_type"] == "Bearer" && value["scope"] == "device:sync",
        "invalid grant scope"
    );
    ensure!(value["cloud_url"] == cloud, "mismatched cloud issuer");
    let original = tokio::fs::read(&w.settings).await?;
    let mut settings: Value = serde_json::from_slice(&original)?;
    ensure!(
        settings["subscription_url"]
            .as_str()
            .unwrap_or("")
            .is_empty()
            && settings["device_token"].as_str().unwrap_or("").is_empty(),
        "binding changed externally"
    );
    settings.as_object_mut().unwrap().remove("subscription_url");
    settings["cloud_url"] = json!(cloud);
    settings["device_token"] = json!(token);
    settings["identity_name"] = value["identity_name"].clone();
    settings["device_id"] = value["device_id"].clone();
    // Sibling temp file, mode 0600, sync and atomic rename. Never truncate the live file.
    let temporary = w
        .settings
        .with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = async {
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary).await?;
        use tokio::io::AsyncWriteExt;
        file.write_all(&serde_json::to_vec_pretty(&settings)?)
            .await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temporary, &w.settings).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result?;
    Ok(cloud.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn local_ui_checks_session_origin_and_preserves_settings() {
        let dir =
            std::env::temp_dir().join(format!("camofy-pairing-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("agent.json");
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = probe.local_addr().unwrap();
        drop(probe);
        let original = json!({"mihomo":"not-needed-before-binding","data_dir":dir,"web_listen":address.to_string(),"dns_redirect":false,"custom":"preserved"});
        tokio::fs::write(&path, serde_json::to_vec(&original).unwrap())
            .await
            .unwrap();
        let settings: Settings = serde_json::from_value(original.clone()).unwrap();
        let _rx = start(&settings, path.clone(), super::super::control::channel().0)
            .await
            .unwrap();
        let base = format!("http://{address}");
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let root = http.get(&base).send().await.unwrap();
        assert_eq!(root.status(), 307);
        assert_eq!(root.headers()["location"], "/bind");
        let page = http.get(format!("{base}/bind")).send().await.unwrap();
        assert_eq!(page.status(), 200);
        let cookie = page.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let html = page.text().await.unwrap();
        assert!(html.contains("<details>"));
        assert!(html.contains(CLOUD));
        assert!(html.contains("local-admin-key"));
        let denied = http
            .post(format!("{base}/api/pair/start"))
            .header("cookie", &cookie)
            .header("origin", "https://evil.example")
            .json(&json!({"device_name":"test"}))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 403);
        assert_eq!(
            http.get(format!("{base}/api/pair/status"))
                .header("host", "evil.example")
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        assert_eq!(
            http.get(format!("{base}/bind/complete?state={}", "a".repeat(64)))
                .header("cookie", &cookie)
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
        let (tx, _) = watch::channel(None);
        let (local, mut actions) = super::super::control::channel();
        let web = Web {
            key_hash: camofy::digest("test-local-key"),
            admins: Arc::new(Mutex::new(vec![(
                "test-admin".into(),
                Instant::now() + Duration::from_secs(60),
            )])),
            login_attempt: Arc::new(Mutex::new(Instant::now() - Duration::from_secs(2))),
            settings: path.clone(),
            http,
            inner: Arc::new(Mutex::new(Inner {
                bound: false,
                cloud: CLOUD.into(),
                identity: String::new(),
                flow: None,
                phase: "idle".into(),
            })),
            tx,
            local,
        };
        let token = "b".repeat(64);
        let grant = json!({"access_token":token,"token_type":"Bearer","scope":"device:sync","cloud_url":CLOUD,"identity_name":"test","device_id":uuid::Uuid::new_v4()});
        let mut bad = grant.clone();
        bad["cloud_url"] = json!("https://evil.example");
        assert!(save_binding(&web, CLOUD, &bad).await.is_err());
        assert_eq!(
            serde_json::from_slice::<Value>(&tokio::fs::read(&path).await.unwrap()).unwrap(),
            original
        );
        save_binding(&web, CLOUD, &grant).await.unwrap();
        let saved: Value = serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
        assert_eq!(saved["custom"], "preserved");
        assert_eq!(saved["dns_redirect"], false);
        assert_eq!(saved["device_token"], grant["access_token"]);
        assert!(saved.get("subscription_url").is_none());
        web.inner.lock().await.bound = true;
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:3000".parse().unwrap());
        headers.insert(header::ORIGIN, "http://127.0.0.1:3000".parse().unwrap());
        headers.insert(
            header::COOKIE,
            format!("{cookie}; camofy_admin=test-admin")
                .parse()
                .unwrap(),
        );
        assert!(
            control(
                State(web.clone()),
                headers.clone(),
                Json(Action {
                    action: "stop".into()
                })
            )
            .await
            .is_err()
        );
        let csrf = session(&headers).unwrap().to_string();
        headers.insert("x-camofy-csrf", csrf.parse().unwrap());
        assert_eq!(
            control(
                State(web.clone()),
                headers,
                Json(Action {
                    action: "stop".into()
                })
            )
            .await
            .unwrap(),
            StatusCode::ACCEPTED
        );
        assert!(
            matches!(actions.recv().await.unwrap(),super::super::control::Request::Core(action) if action=="stop")
        );
        assert!(
            save_binding(&web, CLOUD, &grant).await.is_err(),
            "never overwrite an existing binding"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
    #[test]
    fn rejects_rebinding_hosts_and_unsafe_cloud_urls() {
        let mut h = HeaderMap::new();
        for host in [
            "evil.example:3000",
            "192.168.1.1@evil.example",
            "8.8.8.8:3000",
            "192.168.1.1/evil",
        ] {
            h.insert(header::HOST, host.parse().unwrap());
            assert!(origin(&h).is_none());
        }
        h.insert(header::HOST, "192.168.1.1:3000".parse().unwrap());
        assert_eq!(origin(&h).unwrap(), "http://192.168.1.1:3000");
        assert_eq!(cloud_origin("https://camofy.app/").unwrap(), CLOUD);
        for cloud in [
            "http://cloud.example",
            "https://user:pass@cloud.example",
            "https://cloud.example/path",
            "https://cloud.example/?token=x",
        ] {
            assert!(cloud_origin(cloud).is_err());
        }
    }
}
