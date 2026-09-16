//! Supplier credentials live only inside encrypted resource data. No provider response is logged.
use crate::{App, Error, auth, security};
use anyhow::{Result, ensure};
use axum::{Json, extract::State, http::HeaderMap};
use serde_json::{Value, json};
use std::net::{IpAddr, SocketAddr};
use uuid::Uuid;

const SECRETS: &[&str] = &["url", "extract_url", "whitelist_key"];

pub fn redact(data: &mut Value) {
    for key in SECRETS {
        data.as_object_mut().unwrap().remove(*key);
    }
    data.as_object_mut().unwrap().remove("egress_preview");
}

fn ipv4(text: &str) -> Result<IpAddr> {
    let ip: IpAddr = text
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid server IPv4 response"))?;
    ensure!(
        ip.is_ipv4() && security::public_ip(ip),
        "server egress must be a public IPv4"
    );
    Ok(ip)
}

async fn egress() -> Result<String> {
    let first = url::Url::parse("https://api.ipify.org")?;
    let second = url::Url::parse("https://ipv4.icanhazip.com")?;
    let (a, b) = tokio::try_join!(
        security::fetch_text(&first, false),
        security::fetch_text(&second, false)
    )?;
    let a = ipv4(&a)?;
    ensure!(
        a == ipv4(&b)?,
        "server egress checks disagree; check NAT/routing before authorizing"
    );
    Ok(a.to_string())
}

pub async fn preview(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, true).await?;
    auth::rate(&app, format!("egress:{user}"), 10, 60).await?;
    let ip = egress().await.map_err(|e| Error::bad(e.to_string()))?;
    let expires = crate::now() + 300;
    let proof = app
        .vault
        .seal(&json!({"purpose":"xiequ-egress", "user":user, "ip":ip, "expires":expires}))?;
    Ok(Json(json!({"ip":ip, "expires_at":expires, "proof":proof,
        "sources":["api.ipify.org", "ipv4.icanhazip.com"]})))
}

fn verified_preview(vault: &security::Vault, user: Uuid, proof: Value) -> Result<String> {
    let p = vault
        .open(proof)
        .map_err(|_| anyhow::anyhow!("preview invalid; preview the server IP again"))?;
    ensure!(
        p["purpose"] == "xiequ-egress"
            && p["user"] == user.to_string()
            && p["expires"].as_u64().is_some_and(|t| t > crate::now()),
        "preview expired or belongs to another account; preview again"
    );
    Ok(ipv4(p["ip"].as_str().unwrap_or(""))?.to_string())
}

fn extraction_url(data: &Value) -> Result<url::Url> {
    let u = url::Url::parse(data["extract_url"].as_str().unwrap_or(""))
        .map_err(|_| anyhow::anyhow!("携趣提取接口 URL 无效"))?;
    ensure!(
        ["http", "https"].contains(&u.scheme())
            && u.host_str() == Some("api.xiequ.cn")
            && u.path() == "/VAD/GetIp.aspx"
            && u.port().is_none()
            && u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none(),
        "请使用携趣短效代理 api.xiequ.cn/VAD/GetIp.aspx 提取链接"
    );
    let mut params = std::collections::HashMap::new();
    for (key, value) in u.query_pairs() {
        ensure!(
            params
                .insert(key.into_owned(), value.into_owned())
                .is_none(),
            "提取接口不允许重复参数"
        );
    }
    ensure!(
        params.get("act").map(String::as_str) == Some("get")
            && params.get("num").map(String::as_str) == Some("1")
            && params.get("uid").map(String::as_str) == data["whitelist_uid"].as_str()
            && params.get("vkey").is_some_and(|x| !x.is_empty()),
        "提取接口必须 act=get、num=1，且 uid 与白名单账号一致"
    );
    Ok(u)
}

/// Normalize before any network writes. Switching supplier clears irrelevant secrets.
pub fn normalize(data: &mut Value, old: Option<&Value>) -> Result<()> {
    let supplier = data["provider"].as_str().unwrap_or("static").to_string();
    ensure!(
        ["static", "xiequ"].contains(&supplier.as_str()),
        "unknown proxy provider"
    );
    data["provider"] = json!(supplier);
    let same = old.is_some_and(|o| o["provider"].as_str().unwrap_or("static") == supplier);
    for key in SECRETS {
        if data[*key].as_str().is_none_or(str::is_empty) {
            data[*key] = if same {
                old.unwrap()[*key].clone()
            } else {
                Value::Null
            };
        }
    }
    if supplier == "static" {
        let u = url::Url::parse(data["url"].as_str().unwrap_or(""))
            .map_err(|_| anyhow::anyhow!("invalid proxy URL"))?;
        ensure!(
            ["http", "https", "socks5"].contains(&u.scheme()) && u.host_str().is_some(),
            "supported proxies: http, https, socks5"
        );
        data["endpoint"] = json!(format!(
            "{}://{}:{}",
            u.scheme(),
            u.host_str().unwrap(),
            u.port_or_known_default().unwrap_or(1080)
        ));
        for key in [
            "extract_url",
            "whitelist_key",
            "whitelist_uid",
            "whitelist_ip",
            "whitelist_at",
            "egress_preview",
            "protocol",
        ] {
            data.as_object_mut().unwrap().remove(key);
        }
    } else {
        ensure!(
            data["whitelist_uid"].as_str().is_some_and(|s| !s.is_empty()
                && s.len() <= 30
                && s.bytes().all(|b| b.is_ascii_digit()))
                && data["whitelist_key"].as_str().is_some_and(|s| !s.is_empty()
                    && s.len() <= 128
                    && s.bytes().all(|b| b.is_ascii_alphanumeric())),
            "携趣白名单 uid / ukey 无效"
        );
        extraction_url(data)?;
        let protocol = data["protocol"].as_str().unwrap_or("http");
        ensure!(
            ["http", "socks5"].contains(&protocol),
            "携趣代理协议必须是 HTTP 或 SOCKS5"
        );
        data["protocol"] = json!(protocol);
        data["endpoint"] = json!("携趣 · 短效代理 · 每次刷新提取 1 IP");
        data.as_object_mut().unwrap().remove("url");
        // Never trust client-supplied provisioning status.
        for key in ["whitelist_ip", "whitelist_at"] {
            data[key] = if same {
                old.unwrap()[key].clone()
            } else {
                Value::Null
            };
        }
    }
    Ok(())
}

fn whitelisted(body: &str, ip: &str) -> Result<bool> {
    let v: Value =
        serde_json::from_str(body).map_err(|_| anyhow::anyhow!("携趣白名单返回格式错误"))?;
    let list = v["data"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("携趣白名单查询失败，请检查 uid / ukey"))?;
    Ok(list.iter().any(|entry| entry["IP"].as_str() == Some(ip)))
}

async fn whitelist(data: &Value, ip: &str) -> Result<()> {
    let mut u = url::Url::parse("https://op.xiequ.cn/IpWhiteList.aspx")?;
    u.query_pairs_mut()
        .append_pair("uid", data["whitelist_uid"].as_str().unwrap())
        .append_pair("ukey", data["whitelist_key"].as_str().unwrap())
        .append_pair("act", "getjson");
    if whitelisted(&security::fetch_text(&u, false).await?, ip)? {
        return Ok(());
    }
    let mut add = u.clone();
    add.set_query(None);
    add.query_pairs_mut()
        .append_pair("uid", data["whitelist_uid"].as_str().unwrap())
        .append_pair("ukey", data["whitelist_key"].as_str().unwrap())
        .append_pair("act", "add")
        .append_pair("ip", ip)
        .append_pair("meno", "Camofy cloud");
    // An add response alone is not proof. Read back the exact IP; never clear existing entries.
    security::fetch_text(&add, false).await?;
    ensure!(
        whitelisted(&security::fetch_text(&u, false).await?, ip)?,
        "携趣白名单添加未确认，请检查套餐白名单配额后重试"
    );
    Ok(())
}

/// Runs before opening the edit transaction; a second version check fences concurrent saves.
pub async fn provision(app: &App, user: Uuid, data: &mut Value, old: Option<&Value>) -> Result<()> {
    normalize(data, old)?;
    if data["provider"] != "xiequ" {
        return Ok(());
    }
    let changed = old.is_none_or(|o| {
        [
            "provider",
            "extract_url",
            "whitelist_uid",
            "whitelist_key",
            "protocol",
        ]
        .iter()
        .any(|k| o[*k] != data[*k])
    });
    if changed || data["whitelist_ip"].is_null() || data.get("egress_preview").is_some() {
        let proof = data.get("egress_preview").cloned().unwrap_or(Value::Null);
        let ip = verified_preview(&app.vault, user, proof)?;
        ensure!(
            ip == egress().await?,
            "服务器出口 IP 已变化，请重新预览并确认"
        );
        whitelist(data, &ip).await?;
        data["whitelist_ip"] = json!(ip);
        data["whitelist_at"] = json!(crate::now());
    }
    data.as_object_mut().unwrap().remove("egress_preview");
    Ok(())
}

pub fn parse_proxy(body: &str, protocol: &str, private: bool) -> Result<String> {
    let address = if body.trim_start().starts_with('{') {
        let v: Value =
            serde_json::from_str(body).map_err(|_| anyhow::anyhow!("携趣提取响应 JSON 无效"))?;
        // success may be the string "true" even when code=-1; code is authoritative.
        ensure!(
            v["code"] == 0 || v["code"] == "0",
            "携趣提取失败，请检查白名单、密钥、套餐余额或提取频率"
        );
        let rows = v["data"]
            .as_array()
            .filter(|x| x.len() == 1)
            .ok_or_else(|| anyhow::anyhow!("携趣提取必须返回一个代理 IP"))?;
        let p = &rows[0];
        let port = p["Port"]
            .as_u64()
            .map(|x| x.to_string())
            .or_else(|| p["Port"].as_str().map(str::to_string))
            .unwrap_or_default();
        format!("{}:{}", p["IP"].as_str().unwrap_or(""), port)
    } else {
        body.trim().to_string()
    };
    let addr: SocketAddr = address
        .parse()
        .map_err(|_| anyhow::anyhow!("携趣返回的代理 IP:端口 无效"))?;
    ensure!(
        addr.is_ipv4() && addr.port() > 0 && (private || security::public_ip(addr.ip())),
        "携趣返回了非公网代理地址"
    );
    ensure!(
        ["http", "socks5"].contains(&protocol),
        "invalid provider proxy protocol"
    );
    Ok(format!("{protocol}://{addr}"))
}

pub async fn resolve(data: &Value, private: bool) -> Result<String> {
    if data["provider"].as_str().unwrap_or("static") == "static" {
        return data["url"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("proxy URL missing"));
    }
    ensure!(
        data["provider"] == "xiequ" && data["whitelist_ip"].as_str().is_some(),
        "携趣代理尚未确认服务器白名单"
    );
    let u = extraction_url(data)?;
    extract(&u, data["protocol"].as_str().unwrap_or("http"), private).await
}

/// Shared across replicas and profiles; at most five admissions per one-second window.
/// Two adjacent windows still fit the supplier's recommended ten requests/second.
pub async fn extraction_slot(app: &App, data: &Value) -> Result<()> {
    if data["provider"] != "xiequ" {
        return Ok(());
    }
    let key = format!(
        "xiequ-extract:{}",
        camofy::digest(data["whitelist_uid"].as_str().unwrap_or(""))
    );
    for _ in 0..4 {
        match auth::rate(app, key.clone(), 5, 1).await {
            Ok(()) => return Ok(()),
            Err(e) if e.status == axum::http::StatusCode::TOO_MANY_REQUESTS => {
                tokio::time::sleep(std::time::Duration::from_millis(1100)).await
            }
            Err(_) => anyhow::bail!("proxy extraction admission unavailable"),
        }
    }
    anyhow::bail!("携趣提取排队超时，请稍后刷新")
}

async fn extract(u: &url::Url, protocol: &str, private: bool) -> Result<String> {
    let body = if u.host_str() == Some("api.xiequ.cn") {
        let addrs = match security::addresses(u, false).await {
            Ok(addrs) if addrs.iter().any(SocketAddr::is_ipv4) => addrs,
            _ => {
                // Vendor geo-DNS returns loopback to overseas resolvers. Obtain its CN CDN
                // view over authenticated HTTPS; never hardcode CDN IPs or modify global DNS.
                // This query contains only the fixed public hostname, never API credentials.
                let dns = url::Url::parse(
                    "https://dns.google/resolve?name=api.xiequ.cn&type=A&edns_client_subnet=223.5.5.0/24",
                )?;
                let response = security::fetch_text(&dns, false).await?;
                dns_addresses(&response, u.port_or_known_default().unwrap_or(80))?
            }
        };
        security::fetch_text_at(u, addrs).await?
    } else {
        security::fetch_text(u, private).await?
    };
    parse_proxy(&body, protocol, private)
}

fn dns_addresses(body: &str, port: u16) -> Result<Vec<SocketAddr>> {
    let v: Value =
        serde_json::from_str(body).map_err(|_| anyhow::anyhow!("携趣域名解析响应无效"))?;
    ensure!(
        v["Status"] == 0
            && v["Question"][0]["name"]
                .as_str()
                .is_some_and(|s| s.trim_end_matches('.') == "api.xiequ.cn")
            && v["Question"][0]["type"] == 1,
        "携趣域名解析失败"
    );
    let answers = v["Answer"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("携趣域名缺少公网解析"))?;
    // Accept A records only on the returned CNAME chain, not unrelated additional records.
    let mut names = std::collections::HashSet::from(["api.xiequ.cn".to_string()]);
    for _ in 0..8 {
        for entry in answers {
            if entry["type"] == 5
                && entry["name"]
                    .as_str()
                    .is_some_and(|s| names.contains(s.trim_end_matches('.')))
                && let Some(name) = entry["data"].as_str()
            {
                names.insert(name.trim_end_matches('.').to_string());
            }
        }
    }
    let mut ips = Vec::new();
    for entry in answers {
        if entry["type"] == 1
            && entry["name"]
                .as_str()
                .is_some_and(|s| names.contains(s.trim_end_matches('.')))
        {
            ips.push(SocketAddr::new(
                ipv4(entry["data"].as_str().unwrap_or(""))?,
                port,
            ));
        }
    }
    ensure!(!ips.is_empty(), "携趣域名缺少公网 IPv4 解析");
    Ok(ips)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regional_dns_only_accepts_public_addresses_on_the_cname_chain() {
        let mut d = json!({"Status":0,"Question":[{"name":"api.xiequ.cn.","type":1}],"Answer":[
            {"name":"api.xiequ.cn.","type":5,"data":"cdn.example."},
            {"name":"cdn.example.","type":1,"data":"8.8.8.8"},
            {"name":"unrelated.example.","type":1,"data":"127.0.0.1"}]});
        assert_eq!(
            dns_addresses(&d.to_string(), 80).unwrap(),
            vec!["8.8.8.8:80".parse::<SocketAddr>().unwrap()]
        );
        d["Answer"][1]["data"] = json!("127.0.0.1");
        assert!(dns_addresses(&d.to_string(), 80).is_err());
        d["Question"][0]["name"] = json!("other.example.");
        assert!(dns_addresses(&d.to_string(), 80).is_err());
    }
    #[test]
    fn preview_is_tenant_bound_authenticated_expiring_and_public() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let vault = security::Vault::new(&STANDARD.encode([18; 32])).unwrap();
        let user = Uuid::new_v4();
        let valid = json!({"purpose":"xiequ-egress", "user":user, "ip":"8.8.8.8", "expires":crate::now()+300});
        let proof = vault.seal(&valid).unwrap();
        assert_eq!(
            verified_preview(&vault, user, proof.clone()).unwrap(),
            "8.8.8.8"
        );
        assert!(verified_preview(&vault, Uuid::new_v4(), proof).is_err());
        for (key, value) in [
            ("purpose", json!("other")),
            ("expires", json!(crate::now() - 1)),
            ("ip", json!("127.0.0.1")),
        ] {
            let mut bad = valid.clone();
            bad[key] = value;
            assert!(verified_preview(&vault, user, vault.seal(&bad).unwrap()).is_err());
        }
        assert!(verified_preview(&vault, user, valid).is_err());
    }

    #[tokio::test]
    async fn every_extraction_is_fresh_and_used_without_direct_fallback() {
        use axum::{Router, routing::get};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let hits = Arc::new(AtomicUsize::new(0));
        let mut proxies = Vec::new();
        let mut servers = Vec::new();
        for index in 0..2 {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            proxies.push(l.local_addr().unwrap());
            let hits = hits.clone();
            servers.push(tokio::spawn(
                axum::serve(
                    l,
                    Router::new().fallback(get(move || {
                        hits.fetch_add(1, Ordering::SeqCst);
                        async move { format!("rules: ['DOMAIN,proxy-{index}.test,DIRECT']") }
                    })),
                )
                .into_future(),
            ));
        }
        let count = Arc::new(AtomicUsize::new(0));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let u =
            url::Url::parse(&format!("http://{}/?vkey=SECRET", l.local_addr().unwrap())).unwrap();
        let extraction_server = tokio::spawn(
            axum::serve(
                l,
                Router::new().fallback(get({
                    let count = count.clone();
                    move || {
                        let index = count.fetch_add(1, Ordering::SeqCst);
                        let p = proxies[index.min(1)];
                        async move {
                            Json(if index < 2 {
                                json!({"code":0,"data":[{"IP":p.ip().to_string(),"Port":p.port()}]})
                            } else {
                                json!({"code":-1,"success":"true","msg":"SECRET"})
                            })
                        }
                    }
                })),
            )
            .into_future(),
        );
        for index in 0..2 {
            let endpoint = extract(&u, "http", true).await.unwrap();
            let yaml = security::fetch("http://127.0.0.1:1/source", Some(&endpoint), true)
                .await
                .unwrap();
            assert!(yaml.contains(&format!("proxy-{index}")));
        }
        assert!(extract(&u, "http", true).await.is_err());
        assert_eq!(count.load(Ordering::SeqCst), 3);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert!(security::fetch_text(&u, false).await.is_err());
        extraction_server.abort();
        for s in servers {
            s.abort();
        }
    }

    #[tokio::test]
    async fn provider_transport_rejects_redirects_large_bodies_and_sanitizes_urls() {
        use axum::{Router, response::Redirect, routing::get};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let server = tokio::spawn(
            axum::serve(
                l,
                Router::new()
                    .route(
                        "/redirect",
                        get(|| async { Redirect::temporary("http://127.0.0.1:1/SECRET") }),
                    )
                    .route("/large", get(|| async { "x".repeat(65537) })),
            )
            .into_future(),
        );
        for path in ["redirect", "large", "missing"] {
            let error = security::fetch_text(
                &url::Url::parse(&format!("{base}/{path}?vkey=SECRET")).unwrap(),
                true,
            )
            .await
            .unwrap_err()
            .to_string();
            assert!(!error.contains("SECRET") && !error.contains(&base));
        }
        server.abort();
    }
    #[test]
    fn extraction_formats_and_errors() {
        assert_eq!(
            parse_proxy(
                r#"{"code":0,"success":"true","data":[{"IP":"8.8.8.8","Port":8888}]}"#,
                "http",
                false
            )
            .unwrap(),
            "http://8.8.8.8:8888"
        );
        assert_eq!(
            parse_proxy("8.8.4.4:1080\r\n", "socks5", false).unwrap(),
            "socks5://8.8.4.4:1080"
        );
        for s in [
            r#"{"code":-1,"success":"true","msg":"SECRET","data":""}"#,
            "127.0.0.1:80",
            "192.168.1.1:80",
            "8.8.8.8:0",
            "user:password@8.8.8.8:80",
            "8.8.8.8:80\n8.8.4.4:80",
        ] {
            let e = parse_proxy(s, "http", false).unwrap_err().to_string();
            assert!(!e.contains("SECRET") && !e.contains("password"));
        }
        assert!(whitelisted(r#"{"data":[{"IP":"8.8.8.8","MEMO":"x"}]}"#, "8.8.8.8").unwrap());
        assert!(!whitelisted(r#"{"data":[{"IP":"8.8.8.80"}]}"#, "8.8.8.8").unwrap());
        assert!(whitelisted(r#"{"code":-1}"#, "8.8.8.8").is_err());
    }
    #[test]
    fn normalization_and_redaction() {
        let mut d = json!({"provider":"xiequ", "whitelist_uid":"123", "whitelist_key":"secret",
            "extract_url":"http://api.xiequ.cn/VAD/GetIp.aspx?act=get&num=1&uid=123&vkey=secret", "whitelist_ip":"8.8.8.8"});
        normalize(&mut d, None).unwrap();
        assert!(d["whitelist_ip"].is_null());
        let old = d.clone();
        redact(&mut d);
        assert!(d.get("whitelist_key").is_none() && d.get("extract_url").is_none());
        normalize(&mut d, Some(&old)).unwrap();
        assert_eq!(d["extract_url"], old["extract_url"]);
        d["extract_url"] =
            json!("http://127.0.0.1/VAD/GetIp.aspx?act=get&num=1&uid=123&vkey=secret");
        assert!(normalize(&mut d, Some(&old)).is_err());
    }
}
