//! Shared transport for explicitly selected proxy-provider APIs, not user subscription URLs.
//! Adapters own their URL allowlist, authentication, DNS policy and response format. This module
//! owns bounded networking; it never retries a credential-bearing request behind the caller's back.
mod dns;
pub use dns::DnsPolicy;

use crate::{retry, security};
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

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

/// API errors may contain the credential-bearing URL. Keep the public failure small; emit
/// connection diagnostics separately, with only host, resolved addresses and OS error fields.
#[derive(Clone, Copy, Debug)]
pub struct Failure {
    pub kind: FailureKind,
    pub delay: Option<Duration>,
}

#[derive(Debug)]
struct TransportContext<'a> {
    host: &'a str,
    port: u16,
    addresses: &'a [SocketAddr],
    phase: &'static str,
}

fn transport_context<'a>(
    target: &'a url::Url,
    addrs: &'a [SocketAddr],
    phase: &'static str,
) -> TransportContext<'a> {
    TransportContext {
        host: target.host_str().unwrap_or(""),
        port: target.port_or_known_default().unwrap_or(0),
        addresses: addrs,
        phase,
    }
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

    fn transport(
        error: anyhow::Error,
        target: &url::Url,
        addrs: &[SocketAddr],
        phase: &'static str,
        started: Instant,
    ) -> Self {
        let kind = match error.downcast_ref::<reqwest::Error>() {
            Some(e) if e.status().is_some() => FailureKind::Http(e.status().unwrap().as_u16()),
            Some(e) if e.is_connect() => FailureKind::Connect,
            Some(e) if e.is_timeout() => FailureKind::Timeout,
            _ => FailureKind::Response,
        };
        let failure = Self {
            kind,
            delay: retry::delay(&error),
        };
        let request = error.downcast_ref::<reqwest::Error>();
        let io = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<std::io::Error>());
        let context = transport_context(target, addrs, phase);
        tracing::warn!(
            host = context.host,
            port = context.port,
            addresses = ?context.addresses,
            phase = context.phase,
            elapsed_ms = started.elapsed().as_millis(),
            code = failure.diagnostic().0,
            http_status = ?request.and_then(reqwest::Error::status).map(|s| s.as_u16()),
            is_connect = request.is_some_and(reqwest::Error::is_connect),
            is_timeout = request.is_some_and(reqwest::Error::is_timeout),
            is_body = request.is_some_and(reqwest::Error::is_body),
            io_kind = ?io.map(std::io::Error::kind),
            os_code = ?io.and_then(std::io::Error::raw_os_error),
            "provider API transport failed"
        );
        failure
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
    get_text_with_headers(target, policy, private, reqwest::header::HeaderMap::new()).await
}

/// Supplier adapters may provide authentication headers, but neither headers nor URL are logged.
pub async fn get_text_with_headers(
    target: &url::Url,
    policy: Option<DnsPolicy>,
    private: bool,
    headers: reqwest::header::HeaderMap,
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
    tracing::info!(
        host = target.host_str().unwrap_or(""),
        addresses = ?addrs,
        "provider API DNS resolution completed"
    );
    let result = get_at(target, addrs, private, Duration::from_secs(12), headers).await;
    // A failed connection must not pin future attempts to a possibly obsolete CDN address.
    if let (Some(policy), Err(error)) = (policy, &result)
        && matches!(error.kind, FailureKind::Connect | FailureKind::Timeout)
    {
        dns::invalidate(target, policy).await;
        tracing::info!(
            host = target.host_str().unwrap_or(""),
            code = error.diagnostic().0,
            "provider DNS cache invalidated after transport failure"
        );
    }
    result
}

async fn get_at(
    target: &url::Url,
    addrs: Vec<SocketAddr>,
    private: bool,
    timeout: Duration,
    headers: reqwest::header::HeaderMap,
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
    let started = Instant::now();
    let request = async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(4)))
            .resolve_to_addrs(target.host_str().unwrap(), &addrs)
            .build()
            .map_err(|e| Failure::transport(e.into(), target, &addrs, "client_build", started))?;
        let mut response = client
            .get(target.clone())
            .headers(headers)
            .send()
            .await
            .map_err(|e| {
                Failure::transport(e.into(), target, &addrs, "connect_or_headers", started)
            })?;
        retry::check_http(&response)
            .map_err(|e| Failure::transport(e, target, &addrs, "http_status", started))?;
        if !response.status().is_success() {
            return Err(Failure::permanent(FailureKind::Http(
                response.status().as_u16(),
            )));
        }
        if response.content_length().is_some_and(|n| n > 65536) {
            tracing::warn!(host = target.host_str().unwrap_or(""), addresses = ?addrs,
                content_length = ?response.content_length(), "provider API response too large");
            return Err(Failure::permanent(FailureKind::Response));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| Failure::transport(e.into(), target, &addrs, "response_body", started))?
        {
            if bytes.len() + chunk.len() > 65536 {
                tracing::warn!(host = target.host_str().unwrap_or(""), addresses = ?addrs,
                    bytes = bytes.len() + chunk.len(), "provider API response exceeded limit");
                return Err(Failure::permanent(FailureKind::Response));
            }
            bytes.extend(chunk);
        }
        tracing::info!(host = target.host_str().unwrap_or(""), addresses = ?addrs,
            elapsed_ms = started.elapsed().as_millis(), bytes = bytes.len(),
            "provider API request completed");
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    };
    tokio::time::timeout(timeout, request).await.map_err(|_| {
        tracing::warn!(host = target.host_str().unwrap_or(""), addresses = ?addrs,
                elapsed_ms = started.elapsed().as_millis(), phase = "request_deadline",
                "provider API request timed out");
        Failure::transient(FailureKind::Timeout)
    })?
}

#[cfg(test)]
mod tests;
