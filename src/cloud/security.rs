use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, OsRng, rand_core::RngCore},
};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::net::{IpAddr, SocketAddr};

#[derive(Clone)]
pub struct Vault(Aes256Gcm, [u8; 32]);
impl Vault {
    pub fn new(key: &str) -> Result<Self> {
        let bytes = STANDARD.decode(key)?;
        ensure!(
            bytes.len() == 32,
            "CAMOFY_ENCRYPTION_KEY must be base64 of 32 random bytes"
        );
        Ok(Self(
            Aes256Gcm::new_from_slice(&bytes)
                .map_err(|_| anyhow::anyhow!("invalid encryption key"))?,
            bytes.as_slice().try_into().unwrap(),
        ))
    }
    pub fn source_fingerprint(&self, user: uuid::Uuid, url: &str) -> String {
        use hmac::{Hmac, Mac};
        let normalized = url::Url::parse(url)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| url.into());
        let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(&self.1).unwrap();
        mac.update(b"camofy-usage-source-v1\0");
        mac.update(user.as_bytes());
        mac.update(normalized.as_bytes());
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
    pub fn seal(&self, v: &Value) -> Result<Value> {
        let mut nonce = [0; 12];
        OsRng.fill_bytes(&mut nonce);
        let cipher = self
            .0
            .encrypt(Nonce::from_slice(&nonce), serde_json::to_vec(v)?.as_slice())
            .map_err(|_| anyhow::anyhow!("encryption failed"))?;
        let mut bytes = nonce.to_vec();
        bytes.extend(cipher);
        Ok(json!({"sealed": STANDARD.encode(bytes)}))
    }
    pub fn open(&self, v: Value) -> Result<Value> {
        let bytes = STANDARD.decode(
            v["sealed"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("invalid encrypted record"))?,
        )?;
        ensure!(bytes.len() >= 28, "invalid encrypted record");
        let plain = self
            .0
            .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
            .map_err(|_| anyhow::anyhow!("decryption failed: check encryption key"))?;
        Ok(serde_json::from_slice(&plain)?)
    }
}

pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let o = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_documentation()
                || o[0] == 0
                || o[0] >= 240
                || (o[0] == 100 && (64..=127).contains(&o[1]))
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
                || (o[0] == 192 && o[1] == 0 && o[2] == 0))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            // Only native global unicast; exclude mapped/translation/transition/private ranges.
            (s[0] & 0xe000) == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && s[1] < 0x0200)
                && !(s[0] == 0x2001 && s[1] == 0x0db8)
        }
    }
}

pub async fn addresses(url: &url::Url, private: bool) -> Result<Vec<SocketAddr>> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("URL needs a host"))?
        .trim_matches(['[', ']']);
    let port = url
        .port_or_known_default()
        .or(if url.scheme().starts_with("socks") {
            Some(1080)
        } else {
            None
        })
        .ok_or_else(|| anyhow::anyhow!("URL needs a port"))?;
    let addrs = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::net::lookup_host((host, port)),
    )
    .await??
    .collect::<Vec<_>>();
    ensure!(
        !addrs.is_empty() && (private || addrs.iter().all(|s| public_ip(s.ip()))),
        "destination is not a public network address"
    );
    Ok(addrs)
}

/// User-controlled subscription names never enter the host's recursive resolver.
/// The fixed HTTPS resolver receives the query; ECS is explicitly disabled.
async fn subscription_addresses(target: &url::Url, private: bool) -> Result<Vec<SocketAddr>> {
    let host = target
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("missing host"))?
        .trim_matches(['[', ']']);
    if private || host.parse::<IpAddr>().is_ok() {
        return addresses(target, private).await;
    }
    let port = target
        .port_or_known_default()
        .ok_or_else(|| anyhow::anyhow!("missing port"))?;
    let query = async |kind: u16| -> Result<Vec<SocketAddr>> {
        let mut u = url::Url::parse("https://dns.google/resolve")?;
        u.query_pairs_mut()
            .append_pair("name", host)
            .append_pair("type", &kind.to_string())
            .append_pair("edns_client_subnet", "0.0.0.0/0");
        dns_result(&fetch_text(&u, false).await?, host, kind, port)
    };
    let (mut a, aaaa) = tokio::try_join!(query(1), query(28))?;
    a.extend(aaaa);
    ensure!(!a.is_empty(), "subscription DNS has no public addresses");
    Ok(a)
}
fn dns_result(body: &str, host: &str, kind: u16, port: u16) -> Result<Vec<SocketAddr>> {
    let v: Value = serde_json::from_str(body)?;
    ensure!(
        v["Status"] == 0
            && v["Question"][0]["type"] == kind
            && v["Question"][0]["name"]
                .as_str()
                .is_some_and(|s| s.trim_end_matches('.').eq_ignore_ascii_case(host)),
        "invalid DNS response"
    );
    let entries = v["Answer"].as_array().cloned().unwrap_or_default();
    let mut names = std::collections::HashSet::from([host.trim_end_matches('.').to_lowercase()]);
    for _ in 0..16 {
        for e in &entries {
            if e["type"] == 5
                && e["name"]
                    .as_str()
                    .is_some_and(|s| names.contains(&s.trim_end_matches('.').to_lowercase()))
                && let Some(s) = e["data"].as_str()
            {
                names.insert(s.trim_end_matches('.').to_lowercase());
            }
        }
    }
    let mut result = Vec::new();
    for e in entries {
        if e["type"] == kind
            && e["name"]
                .as_str()
                .is_some_and(|s| names.contains(&s.trim_end_matches('.').to_lowercase()))
        {
            let ip: IpAddr = e["data"].as_str().unwrap_or("").parse()?;
            ensure!(
                public_ip(ip) && ((kind == 1 && ip.is_ipv4()) || (kind == 28 && ip.is_ipv6())),
                "DNS returned an unsafe address"
            );
            result.push(SocketAddr::new(ip, port));
        }
    }
    Ok(result)
}

/// Small provider/egress responses, never redirects, environment proxies or unpinned DNS.
/// Intentionally sanitizes transport errors: API URLs contain credentials.
pub async fn fetch_text(target: &url::Url, private: bool) -> Result<String> {
    fetch_text_inner(target, private, None).await
}

/// Direct JSON POST to a fixed platform service endpoint (captcha vision). Never uses the
/// platform egress, never follows redirects, pins a validated address and bounds both
/// directions. `private` follows CAMOFY_ALLOW_PRIVATE_EGRESS for local/private deployments.
pub async fn post_json(
    target: &url::Url,
    bearer: Option<&str>,
    body: &Value,
    limit: usize,
    private: bool,
) -> Result<Value> {
    let auth = match bearer {
        Some(key) => Auth::Bearer(key),
        None => Auth::None,
    };
    post_json_with(target, auth, body, limit, private).await
}

/// Same guarantees as [`post_json`], for services that authenticate with HTTP Basic
/// (Zyte API uses the API key as the user name with an empty password).
pub async fn post_json_basic(
    target: &url::Url,
    user: &str,
    password: &str,
    body: &Value,
    limit: usize,
    private: bool,
) -> Result<Value> {
    post_json_with(target, Auth::Basic(user, password), body, limit, private).await
}

enum Auth<'a> {
    None,
    Bearer(&'a str),
    Basic(&'a str, &'a str),
}

async fn post_json_with(
    target: &url::Url,
    auth: Auth<'_>,
    body: &Value,
    limit: usize,
    private: bool,
) -> Result<Value> {
    let request = async {
        ensure!(
            ["http", "https"].contains(&target.scheme())
                && target.host_str().is_some()
                && target.username().is_empty()
                && target.password().is_none(),
            "invalid service URL"
        );
        let addrs = addresses(target, private)
            .await?
            .into_iter()
            .filter(SocketAddr::is_ipv4)
            .collect::<Vec<_>>();
        ensure!(!addrs.is_empty(), "service needs an IPv4 address");
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(45));
        builder = builder.resolve_to_addrs(target.host_str().unwrap(), &addrs);
        let mut request = builder.build()?.post(target.clone()).json(body);
        request = match auth {
            Auth::None => request,
            Auth::Bearer(key) => request.bearer_auth(key),
            Auth::Basic(user, password) => request.basic_auth(user, Some(password)),
        };
        let mut response = request.send().await?;
        crate::retry::check_http(&response)?;
        ensure!(response.status().is_success(), "service HTTP failure");
        ensure!(
            response.content_length().is_none_or(|n| n <= limit as u64),
            "service response too large"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= limit,
                "service response too large"
            );
            bytes.extend(chunk);
        }
        Ok::<_, anyhow::Error>(serde_json::from_slice(&bytes)?)
    };
    tokio::time::timeout(std::time::Duration::from_secs(45), request)
        .await
        .map_err(anyhow::Error::new)
        .and_then(|r| r)
}

pub async fn fetch_text_at(target: &url::Url, pinned: Vec<SocketAddr>) -> Result<String> {
    fetch_text_inner(target, false, Some(pinned)).await
}

async fn fetch_text_inner(
    target: &url::Url,
    private: bool,
    pinned: Option<Vec<SocketAddr>>,
) -> Result<String> {
    let request = async {
        ensure!(
            ["http", "https"].contains(&target.scheme())
                && target.username().is_empty()
                && target.password().is_none(),
            "invalid provider URL"
        );
        // Whitelist authorization is IPv4; API requests and egress checks use that family.
        let resolved = match pinned {
            Some(addrs) => {
                ensure!(
                    !addrs.is_empty()
                        && addrs.iter().all(|a| public_ip(a.ip())
                            && Some(a.port()) == target.port_or_known_default()),
                    "invalid pinned public addresses"
                );
                addrs
            }
            None => addresses(target, private).await?,
        };
        let addrs = resolved
            .into_iter()
            .filter(SocketAddr::is_ipv4)
            .collect::<Vec<_>>();
        ensure!(!addrs.is_empty(), "provider needs an IPv4 address");
        let mut response = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(12))
            .resolve_to_addrs(target.host_str().unwrap(), &addrs)
            .build()?
            .get(target.clone())
            .send()
            .await?;
        crate::retry::check_http(&response)?;
        ensure!(response.status().is_success(), "provider HTTP failure");
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 65536,
                "provider response too large"
            );
            bytes.extend(chunk);
        }
        // Legacy messages may be GBK; required numeric IP and JSON keys are ASCII.
        Ok::<_, anyhow::Error>(String::from_utf8_lossy(&bytes).into_owned())
    };
    tokio::time::timeout(std::time::Duration::from_secs(15), request)
        .await
        .map_err(anyhow::Error::new)
        .and_then(|r| r)
        .map_err(|e| {
            anyhow::Error::new(crate::retry::SafeFailure {
                delay: crate::retry::delay(&e),
            })
        })
}

pub const CLIENT_UA: &str = "clash-verge/camofy-cloud";

/// A client pinned to one validated target address, optionally tunnelled through the
/// platform egress proxy. The bridge task must outlive every request on this client,
/// so it is owned here instead of by a single call.
pub struct Egress {
    client: reqwest::Client,
    _bridge: AbortOnDrop,
}
impl Egress {
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }
}

/// Pin destination and proxy DNS. Redirects disabled to prevent credential leakage and SSRF.
/// For proxy requests use a pinned target IP, retaining Host and TLS SNI through reqwest's resolver.
pub async fn egress_client(
    target: &url::Url,
    proxy: Option<&str>,
    private: bool,
    timeout: std::time::Duration,
) -> Result<Egress> {
    ensure!(
        ["http", "https"].contains(&target.scheme())
            && target.host_str().is_some()
            && target.username().is_empty()
            && target.password().is_none(),
        "target URL must be HTTP(S), without userinfo"
    );
    let host = target.host_str().unwrap().to_string();
    let addrs = subscription_addresses(target, private).await?;
    let mut bridge = None;
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(CLIENT_UA);
    builder = builder.resolve_to_addrs(&host, &addrs);
    if let Some(endpoint) = proxy {
        let p = url::Url::parse(endpoint)?;
        ensure!(
            ["http", "https", "socks5"].contains(&p.scheme()),
            "proxy must use http, https or socks5 (local DNS)"
        );
        let proxy_addrs = addresses(&p, private).await?;
        // Own the proxy handshake so every protocol uses the validated numeric target.
        // The HTTP client retains the original URL for Host, SNI and certificate checks.
        let (local, task) = http_proxy_bridge(p, proxy_addrs[0], target.clone(), addrs[0]).await?;
        bridge = Some(task);
        builder = builder.proxy(reqwest::Proxy::all(format!("http://{local}"))?);
    }
    Ok(Egress {
        client: builder.build()?,
        _bridge: AbortOnDrop(bridge),
    })
}

pub struct Fetched {
    pub content: String,
    pub usage: crate::usage::Snapshot,
}
pub async fn fetch(url: &str, proxy: Option<&str>, private: bool) -> Result<Fetched> {
    let target = url::Url::parse(url)?;
    ensure!(
        ["http", "https"].contains(&target.scheme())
            && target.username().is_empty()
            && target.password().is_none(),
        "subscription URL must be HTTP(S), without userinfo"
    );
    let egress = egress_client(&target, proxy, private, std::time::Duration::from_secs(60)).await?;
    let mut response = egress.client().get(target.clone()).send().await?;
    crate::retry::check_http(&response)?;
    ensure!(
        response.status().is_success(),
        "subscription redirect is not allowed; use its final URL"
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|n| n <= 4 * 1024 * 1024),
        "subscription exceeds 4 MiB"
    );
    let usage = crate::usage::parse_headers(response.headers(), crate::now());
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 4 * 1024 * 1024,
            "subscription exceeds 4 MiB"
        );
        bytes.extend(chunk);
    }
    let text = String::from_utf8(bytes)?;
    Ok(Fetched {
        content: text,
        usage,
    })
}

struct AbortOnDrop(Option<tokio::task::JoinHandle<()>>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(t) = &self.0 {
            t.abort();
        }
    }
}
trait ProxyIo: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> ProxyIo for T {}
async fn http_proxy_bridge(
    proxy: url::Url,
    proxy_addr: SocketAddr,
    target: url::Url,
    target_addr: SocketAddr,
) -> Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let task = tokio::spawn(async move {
        // One bridge serves every request of its client until the client is dropped;
        // each accepted connection still pins the same validated target address.
        loop {
            let (mut local, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => break,
            };
            let proxy = proxy.clone();
            let target = target.clone();
            tokio::spawn(async move {
                let exchange = async {
                    let mut head = Vec::new();
                    while !head.ends_with(b"\r\n\r\n") {
                        ensure!(head.len() < 16384, "proxy request headers too large");
                        head.push(local.read_u8().await?);
                    }
                    let text = std::str::from_utf8(&head)?;
                    let tunnel = text.starts_with("CONNECT ");
                    let mut stream = tokio::net::TcpStream::connect(proxy_addr).await?;
                    let socks = proxy.scheme() == "socks5";
                    if socks {
                        socks_handshake(&mut stream, &proxy, target_addr).await?;
                        if tunnel {
                            local
                                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                                .await?;
                            tokio::io::copy_bidirectional(&mut local, &mut stream).await?;
                            return Ok(());
                        }
                    }
                    let mut upstream: Box<dyn ProxyIo> = if proxy.scheme() == "https" {
                        let roots = tokio_rustls::rustls::RootCertStore::from_iter(
                            webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
                        );
                        let config = tokio_rustls::rustls::ClientConfig::builder()
                            .with_root_certificates(roots)
                            .with_no_client_auth();
                        let connector =
                            tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));
                        let name = tokio_rustls::rustls::pki_types::ServerName::try_from(
                            proxy.host_str().unwrap().to_string(),
                        )?;
                        Box::new(connector.connect(name, stream).await?)
                    } else {
                        Box::new(stream)
                    };
                    let mut pinned = target.clone();
                    pinned
                        .set_ip_host(target_addr.ip())
                        .map_err(|_| anyhow::anyhow!("target IP"))?;
                    let mut request = if tunnel {
                        format!("CONNECT {target_addr} HTTP/1.1\r\nHost: {target_addr}\r\n")
                    } else if socks {
                        let path = &target[url::Position::BeforePath..url::Position::AfterQuery];
                        format!("GET {path} HTTP/1.1\r\n")
                    } else {
                        format!("GET {} HTTP/1.1\r\n", pinned.as_str())
                    };
                    let mut forwarded = Vec::new();
                    for line in text.split("\r\n").skip(1).filter(|s| !s.is_empty()) {
                        let lower = line.to_ascii_lowercase();
                        if lower.starts_with("proxy-authorization:")
                            || lower.starts_with("proxy-connection:")
                            || (tunnel && lower.starts_with("host:"))
                        {
                            continue;
                        }
                        forwarded.push(line);
                    }
                    for line in forwarded {
                        request.push_str(line);
                        request.push_str("\r\n");
                    }
                    if !socks && !proxy.username().is_empty() {
                        let username =
                            percent_encoding::percent_decode_str(proxy.username()).decode_utf8()?;
                        let password =
                            percent_encoding::percent_decode_str(proxy.password().unwrap_or(""))
                                .decode_utf8()?;
                        request.push_str(&format!(
                            "Proxy-Authorization: Basic {}\r\n",
                            STANDARD.encode(format!("{username}:{password}"))
                        ));
                    }
                    request.push_str("\r\n");
                    upstream.write_all(request.as_bytes()).await?;
                    tokio::io::copy_bidirectional(&mut local, &mut upstream).await?;
                    Ok::<(), anyhow::Error>(())
                };
                let _ = tokio::time::timeout(std::time::Duration::from_secs(65), exchange).await;
            });
        }
    });
    Ok((address, task))
}

async fn socks_handshake(
    stream: &mut tokio::net::TcpStream,
    proxy: &url::Url,
    target: SocketAddr,
) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let auth = !proxy.username().is_empty() || proxy.password().is_some();
    stream.write_all(&[5, 1, if auth { 2 } else { 0 }]).await?;
    let mut reply = [0; 2];
    stream.read_exact(&mut reply).await?;
    ensure!(
        reply == [5, if auth { 2 } else { 0 }],
        "SOCKS5 authentication method rejected"
    );
    if auth {
        let user = percent_encoding::percent_decode_str(proxy.username()).collect::<Vec<_>>();
        let pass = percent_encoding::percent_decode_str(proxy.password().unwrap_or(""))
            .collect::<Vec<_>>();
        ensure!(
            !user.is_empty() && user.len() <= 255 && !pass.is_empty() && pass.len() <= 255,
            "invalid SOCKS5 credential length"
        );
        let mut login = vec![1, user.len() as u8];
        login.extend(user);
        login.push(pass.len() as u8);
        login.extend(pass);
        stream.write_all(&login).await?;
        stream.read_exact(&mut reply).await?;
        ensure!(reply == [1, 0], "SOCKS5 authentication failed");
    }
    let mut request = vec![5, 1, 0];
    match target.ip() {
        IpAddr::V4(ip) => {
            request.push(1);
            request.extend(ip.octets());
        }
        IpAddr::V6(ip) => {
            request.push(4);
            request.extend(ip.octets());
        }
    }
    request.extend(target.port().to_be_bytes());
    stream.write_all(&request).await?;
    let mut head = [0; 4];
    stream.read_exact(&mut head).await?;
    ensure!(head[..3] == [5, 0, 0], "SOCKS5 connect rejected");
    let n = match head[3] {
        1 => 4,
        4 => 16,
        3 => stream.read_u8().await? as usize,
        _ => anyhow::bail!("invalid SOCKS5 reply"),
    };
    let mut bound = vec![0; n + 2];
    stream.read_exact(&mut bound).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trusted_dns_validates_question_chain_family_and_public_addresses() {
        let mut d = json!({"Status":0,"Question":[{"name":"example.com.","type":1}],"Answer":[
            {"name":"example.com.","type":5,"data":"cdn.example."},
            {"name":"cdn.example.","type":1,"data":"8.8.8.8"},
            {"name":"unrelated.example.","type":1,"data":"127.0.0.1"}]});
        assert_eq!(
            dns_result(&d.to_string(), "example.com", 1, 443).unwrap(),
            vec!["8.8.8.8:443".parse::<SocketAddr>().unwrap()]
        );
        for ip in [
            "127.0.0.1",
            "169.254.169.254",
            "10.0.0.1",
            "::ffff:127.0.0.1",
            "2001:4860:4860::8888",
        ] {
            d["Answer"][1]["data"] = json!(ip);
            assert!(dns_result(&d.to_string(), "example.com", 1, 443).is_err());
        }
        d["Answer"] = json!([]);
        assert!(
            dns_result(&d.to_string(), "example.com", 1, 443)
                .unwrap()
                .is_empty()
        );
        assert!(dns_result(&d.to_string(), "attacker.example", 1, 443).is_err());
        d["Status"] = json!(3);
        assert!(dns_result(&d.to_string(), "example.com", 1, 443).is_err());
    }
    #[tokio::test]
    async fn socks_proxy_pins_target_and_direct_fetch_rejects_redirects() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = server.accept().await.unwrap();
            assert_eq!(socket.read_u8().await.unwrap(), 5);
            let n = socket.read_u8().await.unwrap();
            let mut methods = vec![0; n as usize];
            socket.read_exact(&mut methods).await.unwrap();
            socket.write_all(&[5, 0]).await.unwrap();
            let mut request = [0; 4];
            socket.read_exact(&mut request).await.unwrap();
            assert_eq!(&request[..3], &[5, 1, 0]);
            // No delegated DNS: the SOCKS request must contain numeric IPv4 or IPv6.
            assert!([1, 4].contains(&request[3]));
            let mut destination = vec![0; if request[3] == 1 { 4 } else { 16 }];
            socket.read_exact(&mut destination).await.unwrap();
            assert_eq!(socket.read_u16().await.unwrap(), 59999);
            socket
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                .await
                .unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                head.push(socket.read_u8().await.unwrap());
            }
            let text = String::from_utf8(head).unwrap();
            assert!(text.to_lowercase().contains("host: localhost:59999"));
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\nproxies: []\n\n").await.unwrap();
        });
        let output = fetch(
            "http://localhost:59999/config",
            Some(&format!("socks5://{address}")),
            true,
        )
        .await
        .unwrap();
        assert!(output.content.contains("proxies"));
        task.await.unwrap();
        assert!(fetch("http://127.0.0.1/config", None, false).await.is_err());
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = server.accept().await.unwrap();
            let mut buf = [0; 4096];
            let _ = socket.read(&mut buf).await;
            socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
        });
        assert!(
            fetch(&format!("http://{address}/config"), None, true)
                .await
                .is_err()
        );
        task.await.unwrap();
    }
    #[test]
    fn network_policy() {
        for s in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
            "64:ff9b::a00:1",
        ] {
            assert!(!public_ip(s.parse().unwrap()), "{s}");
        }
        assert!(public_ip("1.1.1.1".parse().unwrap()));
    }
    #[test]
    fn encryption_authenticates() {
        let vault = Vault::new(&STANDARD.encode([1; 32])).unwrap();
        let data = json!({"password":"secret"});
        let sealed = vault.seal(&data).unwrap();
        assert!(!sealed.to_string().contains("secret"));
        assert_eq!(vault.open(sealed.clone()).unwrap(), data);
        assert!(
            Vault::new(&STANDARD.encode([2; 32]))
                .unwrap()
                .open(sealed)
                .is_err()
        );
    }
}
