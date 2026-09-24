//! Shared transport for explicitly selected proxy-provider APIs, not user subscription URLs.
//! Adapters own their URL allowlist, authentication, DNS policy and response format. This module
//! owns bounded networking; it never retries a credential-bearing request behind the caller's back.
mod dns;
pub use dns::DnsPolicy;

use crate::{retry, security};
use std::{net::SocketAddr, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    DnsTimeout,
    DnsUnavailable,
    DnsInvalid,
    Connect,
    Timeout,
    Http(u16),
    Response,
    Configuration,
}

/// Deliberately has no source error: reqwest errors and supplier bodies can contain API keys.
/// Preserve only the category, numeric HTTP status and safe Retry-After delay.
#[derive(Clone, Copy, Debug)]
pub struct Failure {
    pub kind: FailureKind,
    pub delay: Option<Duration>,
}

impl Failure {
    pub fn permanent(kind: FailureKind) -> Self {
        Self { kind, delay: None }
    }

    fn transient(kind: FailureKind) -> Self {
        Self {
            kind,
            delay: Some(Duration::ZERO),
        }
    }

    fn transport(error: anyhow::Error) -> Self {
        let kind = match error.downcast_ref::<reqwest::Error>() {
            Some(e) if e.status().is_some() => FailureKind::Http(e.status().unwrap().as_u16()),
            Some(e) if e.is_connect() => FailureKind::Connect,
            Some(e) if e.is_timeout() => FailureKind::Timeout,
            _ => FailureKind::Response,
        };
        Self {
            kind,
            delay: retry::delay(&error),
        }
    }

    pub fn diagnostic(&self) -> (&'static str, String) {
        let (code, message) = match self.kind {
            FailureKind::DnsTimeout => ("proxy_dns_timeout", "代理提取接口的 DNS 查询超时"),
            FailureKind::DnsUnavailable => ("proxy_dns_failure", "代理提取接口的 DNS 服务暂不可用"),
            FailureKind::DnsInvalid => ("proxy_dns_invalid", "代理提取接口没有返回有效的公网解析"),
            FailureKind::Connect => ("proxy_extract_connect", "无法连接代理提取接口"),
            FailureKind::Timeout => ("proxy_extract_timeout", "代理提取接口请求超时"),
            FailureKind::Http(status) => {
                return (
                    "proxy_extract_http",
                    format!("代理提取接口返回 HTTP {status}，已保留上次成功配置。"),
                );
            }
            FailureKind::Response => (
                "proxy_extract_response",
                "代理提取接口返回异常结果，请检查供应商配置或服务状态",
            ),
            FailureKind::Configuration => ("proxy_configuration", "代理提取接口配置无效"),
        };
        (code, format!("{message}，已保留上次成功配置。"))
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.diagnostic().1)
    }
}
impl std::error::Error for Failure {}

/// Exactly one API request. Parallelism is restricted to credential-free DNS queries.
pub async fn get_text(
    target: &url::Url,
    policy: Option<DnsPolicy>,
    private: bool,
) -> Result<String, Failure> {
    if !["http", "https"].contains(&target.scheme())
        || target.host_str().is_none()
        || !target.username().is_empty()
        || target.password().is_some()
        || target.fragment().is_some()
    {
        return Err(Failure::permanent(FailureKind::Configuration));
    }
    let addrs = match policy {
        Some(policy) => dns::resolve(target, policy).await?,
        None => security::addresses(target, private)
            .await
            .map_err(|error| {
                if error
                    .downcast_ref::<tokio::time::error::Elapsed>()
                    .is_some()
                {
                    Failure::transient(FailureKind::DnsTimeout)
                } else if error.downcast_ref::<std::io::Error>().is_some() {
                    Failure::transient(FailureKind::DnsUnavailable)
                } else {
                    Failure::permanent(FailureKind::DnsInvalid)
                }
            })?,
    };
    let result = get_at(target, addrs, private, Duration::from_secs(12)).await;
    // A failed connection must not pin future attempts to a possibly obsolete CDN address.
    if let (Some(policy), Err(error)) = (policy, &result)
        && matches!(error.kind, FailureKind::Connect | FailureKind::Timeout)
    {
        dns::invalidate(target, policy).await;
    }
    result
}

async fn get_at(
    target: &url::Url,
    addrs: Vec<SocketAddr>,
    private: bool,
    timeout: Duration,
) -> Result<String, Failure> {
    let addrs: Vec<_> = addrs.into_iter().filter(SocketAddr::is_ipv4).collect();
    if addrs.is_empty()
        || addrs.iter().any(|a| {
            (!private && !security::public_ip(a.ip()))
                || Some(a.port()) != target.port_or_known_default()
        })
    {
        return Err(Failure::permanent(FailureKind::DnsInvalid));
    }
    let request = async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(4)))
            .resolve_to_addrs(target.host_str().unwrap(), &addrs)
            .build()
            .map_err(|e| Failure::transport(e.into()))?;
        let mut response = client
            .get(target.clone())
            .send()
            .await
            .map_err(|e| Failure::transport(e.into()))?;
        retry::check_http(&response).map_err(Failure::transport)?;
        if !response.status().is_success() {
            return Err(Failure::permanent(FailureKind::Http(
                response.status().as_u16(),
            )));
        }
        if response.content_length().is_some_and(|n| n > 65536) {
            return Err(Failure::permanent(FailureKind::Response));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| Failure::transport(e.into()))?
        {
            if bytes.len() + chunk.len() > 65536 {
                return Err(Failure::permanent(FailureKind::Response));
            }
            bytes.extend(chunk);
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    };
    tokio::time::timeout(timeout, request)
        .await
        .map_err(|_| Failure::transient(FailureKind::Timeout))?
}

#[cfg(test)]
mod tests;
