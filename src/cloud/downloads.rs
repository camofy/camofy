//! Public, bounded release distribution. Never forward caller credentials or arbitrary URLs.
use crate::{Error, security};
use axum::{
    Router,
    body::Body,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};

const REPOS: [&str; 3] = [
    "camofy/camofy",
    "MetaCubeX/mihomo",
    "MetaCubeX/meta-rules-dat",
];
const MAX_ASSET: u64 = 256 * 1024 * 1024;
struct Service {
    origin: String,
    client: reqwest::Client,
    token: Option<String>,
    slots: Arc<Semaphore>,
    cache: Mutex<HashMap<String, (Instant, Value)>>,
}
pub fn routes<S: Clone + Send + Sync + 'static>(origin: &str) -> Router<S> {
    let service = Arc::new(Service {
        origin: origin.to_owned(),
        client: reqwest::Client::builder()
            .no_proxy()
            .no_gzip()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .user_agent("Camofy-Release-Distribution")
            .build()
            .expect("valid HTTP client"),
        token: std::env::var("CAMOFY_GITHUB_TOKEN")
            .ok()
            .filter(|s| !s.is_empty()),
        slots: Arc::new(Semaphore::new(4)),
        cache: Mutex::new(HashMap::new()),
    });
    Router::new()
        .route("/install.sh", get(installer))
        .route("/github/*path", get(proxy))
        .route("/downloads/manifest/:arch", get(manifest))
        .with_state(service)
}
fn unavailable() -> Error {
    Error::new(StatusCode::BAD_GATEWAY, "release upstream unavailable")
}
fn busy() -> Error {
    Error::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "download capacity reached; retry later",
    )
}
fn component(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 180
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
}
#[derive(Debug, PartialEq)]
enum Target {
    Latest(String),
    Asset(String),
}
fn target(path: &str) -> Result<Target, Error> {
    let parts: Vec<_> = path.split('/').collect();
    if let ["repos", owner, repo, "releases", "latest"] = parts.as_slice() {
        let repo = format!("{owner}/{repo}");
        if REPOS.contains(&repo.as_str()) {
            return Ok(Target::Latest(repo));
        }
    }
    if let [owner, repo, "releases", kind, tag, asset] = parts.as_slice() {
        let repo = format!("{owner}/{repo}");
        if REPOS.contains(&repo.as_str())
            && component(asset)
            && ((*kind == "download" && component(tag))
                || (*kind == "latest" && *tag == "download"))
        {
            return Ok(Target::Asset(format!("https://github.com/{path}")));
        }
    }
    Err(Error::new(
        StatusCode::FORBIDDEN,
        "only approved repository release endpoints are available",
    ))
}
fn trusted(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "api.github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
                    | "github-releases.githubusercontent.com"
            )
        )
}
fn range(headers: &HeaderMap) -> Result<Option<&str>, Error> {
    let Some(value) = headers.get(header::RANGE) else {
        return Ok(None);
    };
    let s = value
        .to_str()
        .map_err(|_| Error::bad("invalid byte range"))?;
    let valid = s
        .strip_prefix("bytes=")
        .and_then(|v| v.split_once('-'))
        .is_some_and(|(a, b)| {
            !(a.is_empty() && b.is_empty())
                && a.bytes().all(|b| b.is_ascii_digit())
                && b.bytes().all(|b| b.is_ascii_digit())
        });
    if !valid || s.len() > 64 {
        return Err(Error::bad("only one byte range is supported"));
    }
    Ok(Some(s))
}
impl Service {
    async fn request(
        &self,
        method: Method,
        url: &str,
        bytes: Option<&str>,
    ) -> Result<reqwest::Response, Error> {
        let mut url = url::Url::parse(url).map_err(|_| unavailable())?;
        for hop in 0..5 {
            if !trusted(&url) {
                tracing::warn!(hop, host = ?url.host_str(), "release upstream redirect rejected");
                return Err(unavailable());
            }
            let started = Instant::now();
            let host = url.host_str().unwrap_or("");
            let mut request = self
                .client
                .request(method.clone(), url.clone())
                .header(header::ACCEPT_ENCODING, "identity");
            // Tokens are operator-owned, used only on GitHub's API, and never sent to asset hosts.
            if url.host_str() == Some("api.github.com") {
                request = request.header(header::ACCEPT, "application/vnd.github+json");
                if let Some(token) = &self.token {
                    request = request.bearer_auth(token);
                }
            }
            if let Some(bytes) = bytes {
                request = request.header(header::RANGE, bytes);
            }
            let response = request.send().await.map_err(|error| {
                security::log_network_failure(
                    "release_distribution",
                    host,
                    "request_headers",
                    started,
                    &anyhow::Error::new(error),
                );
                unavailable()
            })?;
            tracing::info!(host, hop, method = %method, status = response.status().as_u16(),
                elapsed_ms = started.elapsed().as_millis(), "release upstream responded");
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|h| h.to_str().ok())
                    .ok_or_else(unavailable)?;
                url = url.join(location).map_err(|_| unavailable())?;
                continue;
            }
            return match response.status() {
                StatusCode::OK | StatusCode::PARTIAL_CONTENT => Ok(response),
                StatusCode::NOT_FOUND => Err(Error::new(
                    StatusCode::NOT_FOUND,
                    "release or asset not found",
                )),
                StatusCode::RANGE_NOT_SATISFIABLE => Err(Error::new(
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    "range not satisfiable",
                )),
                StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => Err(Error::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "GitHub rate limit or access restriction; retry later",
                )),
                _ => Err(unavailable()),
            };
        }
        tracing::warn!("release upstream exceeded redirect limit");
        Err(unavailable())
    }
    async fn latest(&self, repo: &str) -> Result<Value, Error> {
        let mut cache = self.cache.lock().await;
        if let Some((time, value)) = cache.get(repo)
            && time.elapsed() < Duration::from_secs(300)
        {
            return Ok(value.clone());
        }
        let _permit = self.slots.clone().try_acquire_owned().map_err(|_| busy())?;
        let started = Instant::now();
        let value = tokio::time::timeout(Duration::from_secs(25), async {
            let mut response = self
                .request(
                    Method::GET,
                    &format!("https://api.github.com/repos/{repo}/releases/latest"),
                    None,
                )
                .await?;
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|error| {
                security::log_network_failure(
                    "release_distribution",
                    "api.github.com",
                    "latest_body",
                    started,
                    &anyhow::Error::new(error),
                );
                unavailable()
            })? {
                if body.len() + chunk.len() > 2 * 1024 * 1024 {
                    return Err(unavailable());
                }
                body.extend_from_slice(&chunk);
            }
            let raw: Value = serde_json::from_slice(&body).map_err(|_| unavailable())?;
            release(&self.origin, repo, &raw)
        })
        .await
        .map_err(|_| {
            tracing::warn!(repo, "release metadata fetch exceeded deadline");
            unavailable()
        })??;
        cache.insert(repo.to_owned(), (Instant::now(), value.clone()));
        tracing::info!(repo, "release metadata cache updated");
        Ok(value)
    }
}
fn release(origin: &str, repo: &str, raw: &Value) -> Result<Value, Error> {
    let tag = raw["tag_name"]
        .as_str()
        .filter(|s| component(s))
        .ok_or_else(unavailable)?;
    let assets = raw["assets"].as_array().ok_or_else(unavailable)?.iter().filter_map(|a| {
        let name = a["name"].as_str().filter(|s| component(s))?;
        Some(json!({"name":name,"size":a["size"],"digest":a["digest"],
            "browser_download_url":format!("{origin}/github/{repo}/releases/download/{tag}/{name}")}))
    }).collect::<Vec<_>>();
    Ok(
        json!({"tag_name":tag,"name":raw["name"],"published_at":raw["published_at"],"assets":assets}),
    )
}
async fn installer(State(service): State<Arc<Service>>) -> Response {
    let script = include_str!("../../install.sh").replace("https://camofy.app", &service.origin);
    (
        [
            (header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=60"),
        ],
        script,
    )
        .into_response()
}
async fn proxy(
    State(service): State<Arc<Service>>,
    Path(path): Path<String>,
    RawQuery(query): RawQuery,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, Error> {
    if query.is_some() {
        return Err(Error::bad(
            "query parameters and client GitHub tokens are not supported",
        ));
    }
    match target(&path)? {
        Target::Latest(repo) => Ok((
            [(header::CACHE_CONTROL, "public, max-age=60")],
            axum::Json(service.latest(&repo).await?),
        )
            .into_response()),
        Target::Asset(url) => {
            let permit = service
                .slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| busy())?;
            let response = service
                .request(method.clone(), &url, range(&headers)?)
                .await?;
            let length = response.content_length();
            if length.is_some_and(|n| n > MAX_ASSET) {
                return Err(unavailable());
            }
            let mut builder = Response::builder()
                .status(response.status())
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header(header::CACHE_CONTROL, "public, max-age=300");
            for key in [
                header::CONTENT_LENGTH,
                header::CONTENT_RANGE,
                header::ACCEPT_RANGES,
                header::ETAG,
                header::LAST_MODIFIED,
            ] {
                if let Some(value) = response.headers().get(&key) {
                    builder = builder.header(key, value);
                }
            }
            if method == Method::HEAD {
                return builder.body(Body::empty()).map_err(|_| unavailable());
            }
            let stream = futures_util::stream::try_unfold(
                (response, permit, 0_u64, Instant::now()),
                |(mut response, permit, total, started)| async move {
                    let host = response.url().host_str().unwrap_or("").to_string();
                    match response.chunk().await.map_err(|error| {
                        security::log_network_failure(
                            "release_distribution",
                            &host,
                            "asset_body",
                            started,
                            &anyhow::Error::new(error),
                        );
                        std::io::Error::other("upstream download interrupted")
                    })? {
                        None => Ok(None),
                        Some(chunk) => {
                            let total = total + chunk.len() as u64;
                            if total > MAX_ASSET {
                                return Err(std::io::Error::other("asset size limit exceeded"));
                            }
                            Ok(Some((chunk, (response, permit, total, started))))
                        }
                    }
                },
            );
            builder
                .body(Body::from_stream(stream))
                .map_err(|_| unavailable())
        }
    }
}
fn asset_line(kind: &str, release: &Value, name: &str) -> Result<String, Error> {
    let asset = release["assets"].as_array().and_then(|items| items.iter().find(|a| a["name"] == name))
        .ok_or_else(|| Error::new(StatusCode::SERVICE_UNAVAILABLE, "latest release has no compatible Agent/core; a new Agent release must be published before installation"))?;
    let hash = asset["digest"]
        .as_str()
        .and_then(|s| s.strip_prefix("sha256:"))
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| {
            Error::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "release asset has no SHA256 digest; refusing unverified installation",
            )
        })?;
    Ok(format!(
        "{kind} {} {hash}\n",
        asset["browser_download_url"]
            .as_str()
            .ok_or_else(unavailable)?
    ))
}
async fn manifest(
    State(service): State<Arc<Service>>,
    Path(arch): Path<String>,
) -> Result<Response, Error> {
    let (target, core) = match arch.as_str() {
        "amd64" => ("x86_64-unknown-linux-musl", "amd64-compatible"),
        "armv7" => ("armv7-unknown-linux-musleabihf", "armv7"),
        _ => return Err(Error::bad("supported architectures: amd64, armv7")),
    };
    let agent = service.latest("camofy/camofy").await?;
    let mut body = asset_line("agent", &agent, &format!("camofy-agent-{target}.tar.gz"))?;
    let mihomo = service.latest("MetaCubeX/mihomo").await?;
    body += &asset_line(
        "mihomo",
        &mihomo,
        &format!(
            "mihomo-linux-{core}-{}.gz",
            mihomo["tag_name"].as_str().ok_or_else(unavailable)?
        ),
    )?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;
    #[test]
    fn restricts_routes_redirects_ranges_and_metadata() {
        assert_eq!(
            target("repos/camofy/camofy/releases/latest").unwrap(),
            Target::Latest("camofy/camofy".into())
        );
        assert!(target("MetaCubeX/mihomo/releases/latest/download/mihomo.gz").is_ok());
        for path in [
            "repos/other/repo/releases/latest",
            "camofy/other/releases/download/v1/a",
            "camofy/camofy/raw/main/foo",
            "camofy/camofy/releases/download/../a",
            "camofy/camofy/releases/download/v1/a?token=x",
        ] {
            assert!(target(path).is_err(), "{path}");
        }
        for url in [
            "http://github.com/a",
            "https://evil.test/a",
            "https://github.com.evil.test/a",
            "https://user@github.com/a",
            "https://github.com:444/a",
            "https://127.0.0.1/a",
        ] {
            assert!(!trusted(&url.parse().unwrap()));
        }
        assert!(trusted(
            &"https://release-assets.githubusercontent.com/x?sig=private"
                .parse()
                .unwrap()
        ));
        let mut headers = HeaderMap::new();
        for value in ["bytes=0-9", "bytes=10-", "bytes=-8"] {
            headers.insert(header::RANGE, value.parse().unwrap());
            assert!(range(&headers).is_ok());
        }
        for value in ["bytes=-", "bytes=0-1,3-4", "items=1-3", "bytes=x-y"] {
            headers.insert(header::RANGE, value.parse().unwrap());
            assert!(range(&headers).is_err());
        }
        let raw = json!({"tag_name":"v1", "assets":[{"name":"file.tar.gz","size":123,"digest":format!("sha256:{}","a".repeat(64)),"browser_download_url":"https://evil.test/","url":"secret"}],"body":"private"});
        let clean = release("https://self.example", "camofy/camofy", &raw).unwrap();
        assert_eq!(
            clean["assets"][0]["browser_download_url"],
            "https://self.example/github/camofy/camofy/releases/download/v1/file.tar.gz"
        );
        assert!(!clean.to_string().contains("secret"));
        assert!(asset_line("agent", &clean, "missing").is_err());
        assert!(
            asset_line("agent", &clean, "file.tar.gz")
                .unwrap()
                .ends_with(&format!("{}\n", "a".repeat(64)))
        );
    }
    #[tokio::test]
    async fn manifest_uses_cached_pinned_assets_and_rejects_old_releases() {
        let service = Arc::new(Service {
            origin: "https://self.example".into(),
            client: reqwest::Client::new(),
            token: None,
            slots: Arc::new(Semaphore::new(0)),
            cache: Mutex::new(HashMap::new()),
        });
        let agent = release(&service.origin, "camofy/camofy", &json!({"tag_name":"v2","assets":[{
            "name":"camofy-agent-armv7-unknown-linux-musleabihf.tar.gz","digest":format!("sha256:{}","a".repeat(64))}]})).unwrap();
        let core = release(
            &service.origin,
            "MetaCubeX/mihomo",
            &json!({"tag_name":"v1","assets":[{
            "name":"mihomo-linux-armv7-v1.gz","digest":format!("sha256:{}","b".repeat(64))}]}),
        )
        .unwrap();
        service
            .cache
            .lock()
            .await
            .insert("camofy/camofy".into(), (Instant::now(), agent));
        service
            .cache
            .lock()
            .await
            .insert("MetaCubeX/mihomo".into(), (Instant::now(), core));
        let response = manifest(State(service.clone()), Path("armv7".into()))
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("https://self.example/github/camofy/camofy/releases/download/v2/"));
        assert!(
            text.contains("https://self.example/github/MetaCubeX/mihomo/releases/download/v1/")
        );
        let error = manifest(State(service.clone()), Path("amd64".into()))
            .await
            .unwrap_err();
        assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
        service.cache.lock().await.clear();
        assert_eq!(
            service.latest("camofy/camofy").await.unwrap_err().status,
            StatusCode::SERVICE_UNAVAILABLE
        );
        let mut raw = json!({"tag_name":"v1","assets":[{"name":"file","digest":null}]});
        assert!(
            asset_line(
                "agent",
                &release(&service.origin, "camofy/camofy", &raw).unwrap(),
                "file"
            )
            .is_err()
        );
        raw["tag_name"] = json!("../../private");
        assert!(release(&service.origin, "camofy/camofy", &raw).is_err());
    }
    #[tokio::test]
    async fn public_installer_and_rejected_requests_need_no_database() {
        let router = routes::<()>("https://self.example");
        for (method, uri, expected) in [
            ("GET", "/install.sh", 200),
            ("HEAD", "/install.sh", 200),
            ("POST", "/github/repos/camofy/camofy/releases/latest", 405),
            ("GET", "/github/repos/evil/repo/releases/latest", 403),
            (
                "GET",
                "/github/repos/camofy/camofy/releases/latest?token=secret",
                400,
            ),
            ("GET", "/downloads/manifest/mips", 400),
        ] {
            let response = router
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), expected, "{uri}");
            if method == "GET" && uri == "/install.sh" {
                let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
                    .await
                    .unwrap();
                let body = std::str::from_utf8(&body).unwrap();
                assert!(body.starts_with("#!/"));
                assert!(body.contains("https://self.example"));
                assert!(!body.contains("mirror.camofy.app"));
            }
        }
    }
}
