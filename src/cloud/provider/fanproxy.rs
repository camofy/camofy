//! FanProxy domestic short-lived IP adapter. The API key is never sent to the allowlist API;
//! the account/signature are never sent to the extraction endpoint.
use crate::{provider_net, security};
use anyhow::{Result, ensure};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};
use std::net::{IpAddr, SocketAddr};

#[derive(Debug)]
pub struct AllowlistRejection {
    pub code: i64,
}

impl std::fmt::Display for AllowlistRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "网帆白名单操作失败（错误码 {}）", self.code)
    }
}
impl std::error::Error for AllowlistRejection {}

const API: &str = "https://api.fanproxy.com/";
const OPENAPI: &str = "https://openapi.fanproxy.com";
// The extraction CDN is regional. OpenAPI's allowlist uses a separate global CDN;
// regional ECS there can select an edge that answers 401 for valid credentials.
const EXTRACT_DNS: provider_net::DnsPolicy = provider_net::DnsPolicy {
    subnet: Some((std::net::Ipv4Addr::new(223, 5, 5, 0), 24)),
};

pub fn validate(data: &mut Value) -> Result<()> {
    ensure!(
        data["extract_key"].as_str().is_some_and(|s| !s.is_empty()
            && s.len() <= 128
            && s.bytes().all(|b| b.is_ascii_alphanumeric()))
            && data["whitelist_account"]
                .as_str()
                .is_some_and(|s| s.len() == 11 && s.bytes().all(|b| b.is_ascii_digit()))
            && data["whitelist_signature"]
                .as_str()
                .is_some_and(|s| !s.is_empty()
                    && s.len() <= 128
                    && s.bytes().all(|b| b.is_ascii_alphanumeric())),
        "网帆提取 Key、账号或白名单签名无效"
    );
    ensure!(
        data["area"].is_null() || data["area"].is_string(),
        "网帆地区编码无效"
    );
    ensure!(
        data["isp"].is_null() || data["isp"].is_string(),
        "网帆运营商无效"
    );
    let area = data["area"].as_str().unwrap_or("").to_owned();
    ensure!(
        area.is_empty() || (area.len() == 6 && area.bytes().all(|b| b.is_ascii_digit())),
        "网帆地区编码必须是 6 位数字，或留空表示全部地区"
    );
    ensure!(
        ["", "电信", "联通", "移动"].contains(&data["isp"].as_str().unwrap_or("")),
        "网帆运营商仅支持不限、电信、联通、移动"
    );
    ensure!(
        data["deduplicate"].is_null() || data["deduplicate"].is_boolean(),
        "网帆去重选项无效"
    );
    if data["deduplicate"].is_null() {
        data["deduplicate"] = json!(true);
    }
    data["area"] = json!(area);
    if data["isp"].is_null() {
        data["isp"] = json!("");
    }
    Ok(())
}

fn extraction_url(data: &Value) -> Result<url::Url> {
    let mut url = url::Url::parse(API)?;
    url.query_pairs_mut()
        .append_pair("key", data["extract_key"].as_str().unwrap())
        .append_pair("count", "1")
        .append_pair("pattern", "json")
        .append_pair("protocol", "2")
        .append_pair("ipv", "4")
        .append_pair("rreg", "true")
        .append_pair("rcity", "true")
        .append_pair("risp", "true")
        .append_pair("rexp", "true");
    if data["deduplicate"] == true {
        url.query_pairs_mut().append_pair("mr", "1");
    }
    if let Some(area) = data["area"]
        .as_str()
        .filter(|s| !s.is_empty() && *s != "000000")
    {
        url.query_pairs_mut().append_pair("area", area);
    }
    if let Some(isp) = data["isp"].as_str().filter(|s| !s.is_empty()) {
        url.query_pairs_mut().append_pair("isp", isp);
    }
    Ok(url)
}

fn parse_proxy(body: &str, private: bool) -> Result<String> {
    let v: Value =
        serde_json::from_str(body).map_err(|_| anyhow::anyhow!("网帆提取响应 JSON 无效"))?;
    let code = v["code"]
        .as_i64()
        .or_else(|| v["code"].as_str().and_then(|s| s.parse().ok()));
    if code != Some(0) || v["success"] == false {
        tracing::warn!(api_code = ?code, "FanProxy extraction rejected");
        anyhow::bail!("网帆提取失败，请检查白名单、套餐、配额或提取间隔");
    }
    let rows = v["data"]
        .as_array()
        .filter(|rows| rows.len() == 1)
        .ok_or_else(|| anyhow::anyhow!("网帆提取必须返回一个代理 IP"))?;
    let row = &rows[0];
    let port = row["port"]
        .as_u64()
        .map(|v| v.to_string())
        .or_else(|| row["port"].as_str().map(str::to_string))
        .unwrap_or_default();
    let address: SocketAddr = format!("{}:{port}", row["ip"].as_str().unwrap_or(""))
        .parse()
        .map_err(|_| anyhow::anyhow!("网帆返回的代理 IP:端口 无效"))?;
    ensure!(
        address.is_ipv4() && address.port() > 0 && (private || security::public_ip(address.ip())),
        "网帆返回了非公网代理地址"
    );
    Ok(format!("http://{address}"))
}

pub async fn extract(data: &Value, private: bool) -> Result<String> {
    let body = provider_net::get_text(&extraction_url(data)?, Some(EXTRACT_DNS), private).await?;
    parse_proxy(&body, private)
        .map_err(|_| provider_net::Failure::permanent(provider_net::FailureKind::Response).into())
}

fn headers(data: &Value) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Api-Key",
        HeaderValue::from_str(data["whitelist_account"].as_str().unwrap())?,
    );
    headers.insert(
        "X-Api-Signature",
        HeaderValue::from_str(data["whitelist_signature"].as_str().unwrap())?,
    );
    headers.insert(
        "X-Api-Timestamp",
        HeaderValue::from_str(&crate::now().to_string())?,
    );
    Ok(headers)
}

fn allowlist_response(body: &str) -> Result<Value> {
    let v: Value =
        serde_json::from_str(body).map_err(|_| anyhow::anyhow!("网帆白名单响应 JSON 无效"))?;
    // The domestic page says code=0, while its example and shared OpenAPI contract say 200.
    let code = v["code"].as_i64();
    if !matches!(code, Some(200 | 0)) || v["success"] == false {
        tracing::warn!(api_code = code.unwrap_or(-1), "FanProxy allowlist rejected");
        return Err(AllowlistRejection {
            code: code.unwrap_or(-1),
        }
        .into());
    }
    Ok(v)
}

async fn listed(data: &Value, ip: &str) -> Result<bool> {
    let url = url::Url::parse(&format!("{OPENAPI}/open-api/open/white/query"))?;
    let body = provider_net::get_text_with_headers(&url, None, false, headers(data)?).await?;
    let response = allowlist_response(&body)?;
    let addresses = response["data"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("网帆白名单查询未返回 IP 列表"))?;
    Ok(addresses
        .iter()
        .any(|value| value.as_str() == Some(ip) || value["ip"].as_str() == Some(ip)))
}

pub async fn whitelist(data: &Value, ip: &str) -> Result<()> {
    let address: IpAddr = ip.parse()?;
    ensure!(
        address.is_ipv4() && security::public_ip(address),
        "白名单出口必须是公网 IPv4"
    );
    if listed(data, ip).await? {
        return Ok(());
    }
    let mut url = url::Url::parse(&format!("{OPENAPI}/open-api/open/white/add"))?;
    url.query_pairs_mut().append_pair("ip", ip);
    let body = provider_net::get_text_with_headers(&url, None, false, headers(data)?).await?;
    // A concurrent add can report 'already present'; read back rather than trusting either code.
    if let Err(error) = allowlist_response(&body) {
        let code = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v["code"].as_i64());
        if !matches!(code, Some(1208 | 1209)) {
            return Err(error);
        }
    }
    ensure!(
        listed(data, ip).await?,
        "网帆白名单添加未确认，请检查配额或地区限制"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Value {
        json!({"extract_key":"SECRET123", "whitelist_account":"00000000000",
            "whitelist_signature":"abcdef0123456789abcdef0123456789", "area":"110100",
            "isp":"电信", "deduplicate":true})
    }

    #[test]
    fn fixed_protocol_and_user_choices_are_encoded_without_exposing_secrets() {
        let mut config = data();
        validate(&mut config).unwrap();
        let url = extraction_url(&config).unwrap();
        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(url.host_str(), Some("api.fanproxy.com"));
        for (key, expected) in [
            ("count", "1"),
            ("pattern", "json"),
            ("protocol", "2"),
            ("ipv", "4"),
            ("mr", "1"),
            ("area", "110100"),
            ("isp", "电信"),
        ] {
            assert_eq!(params.get(key).map(String::as_str), Some(expected));
        }
        config["deduplicate"] = json!(false);
        config["area"] = json!("");
        config["isp"] = json!("");
        validate(&mut config).unwrap();
        let params: std::collections::HashMap<_, _> = extraction_url(&config)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
        assert!(
            !params.contains_key("mr")
                && !params.contains_key("area")
                && !params.contains_key("isp")
        );
    }

    #[test]
    fn response_and_allowlist_require_authoritative_success() {
        assert_eq!(
            parse_proxy(r#"{"code":0,"data":[{"ip":"8.8.8.8","port":8080}]}"#, false).unwrap(),
            "http://8.8.8.8:8080"
        );
        assert_eq!(
            parse_proxy(
                r#"{"code":"0","success":true,"data":[{"ip":"8.8.4.4","port":"80"}]}"#,
                false
            )
            .unwrap(),
            "http://8.8.4.4:80"
        );
        for body in [
            r#"{"code":1303,"message":"SECRET"}"#,
            r#"{"code":0,"success":false,"data":[{"ip":"8.8.8.8","port":80}]}"#,
            r#"{"code":0,"data":[{"ip":"127.0.0.1","port":80}]}"#,
        ] {
            assert!(
                !parse_proxy(body, false)
                    .unwrap_err()
                    .to_string()
                    .contains("SECRET")
            );
        }
        for body in [
            r#"{"code":200,"success":true,"data":[]}"#,
            r#"{"code":0,"data":[]}"#,
        ] {
            assert!(allowlist_response(body).is_ok());
        }
        for body in [
            r#"{"code":1212,"success":false,"message":"SECRET"}"#,
            r#"{"code":200,"success":false}"#,
        ] {
            assert!(
                !allowlist_response(body)
                    .unwrap_err()
                    .to_string()
                    .contains("SECRET")
            );
        }
        assert_eq!(
            allowlist_response(r#"{"code":1212,"success":false}"#)
                .unwrap_err()
                .downcast_ref::<AllowlistRejection>()
                .unwrap()
                .code,
            1212
        );
    }

    #[test]
    fn unsupported_user_options_are_rejected() {
        for (field, value) in [
            ("area", json!("1101")),
            ("isp", json!("其他")),
            ("deduplicate", json!("maybe")),
        ] {
            let mut config = data();
            config[field] = value;
            assert!(validate(&mut config).is_err());
        }
    }
}
