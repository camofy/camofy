//! Bounded retries. Error classification never needs provider bodies or secret URLs.
use std::time::Duration;

pub const ATTEMPTS: usize = 3;
pub const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(100);
/// A resolved panel subscription stays fetchable for ten minutes after activation; retries
/// inside one refresh reuse it instead of paying for another login and captcha.
pub const PANEL_REUSE: Duration = Duration::from_secs(480);

#[derive(Debug)]
pub struct SafeFailure {
    pub delay: Option<Duration>,
}
impl std::fmt::Display for SafeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("provider request failed (network, HTTP status or invalid response)")
    }
}
impl std::error::Error for SafeFailure {}

pub fn transient_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

/// None means a permanent/unknown failure. Do not retry unsafe destinations,
/// invalid credentials, malformed provider responses or invalid YAML by guessing.
pub fn delay(error: &anyhow::Error) -> Option<Duration> {
    if let Some(e) = error.downcast_ref::<SafeFailure>() {
        return e.delay;
    }
    if error.chain().any(|e| e.is::<tokio_rustls::rustls::Error>()) {
        return None;
    }
    if let Some(e) = error.downcast_ref::<reqwest::Error>() {
        if let Some(status) = e.status() {
            return transient_status(status.as_u16()).then_some(Duration::ZERO);
        }
        if e.is_timeout() || e.is_connect() || e.is_body() || e.is_request() {
            return Some(Duration::ZERO);
        }
    }
    if error
        .downcast_ref::<tokio::time::error::Elapsed>()
        .is_some()
    {
        return Some(Duration::ZERO);
    }
    if error.chain().any(|e| {
        e.downcast_ref::<std::io::Error>().is_some_and(|e| {
            matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::Interrupted
            )
        })
    }) {
        return Some(Duration::ZERO);
    }
    None
}

pub fn check_http(response: &reqwest::Response) -> anyhow::Result<()> {
    if let Err(error) = response.error_for_status_ref() {
        let mut wait = transient_status(response.status().as_u16()).then_some(Duration::ZERO);
        if let Some(value) = response.headers().get(reqwest::header::RETRY_AFTER) {
            // Honor delta seconds. Unknown/date-form values conservatively stop
            // this refresh instead of retrying earlier than the server permits.
            wait = wait.and_then(|_| {
                value
                    .to_str()
                    .ok()?
                    .parse::<u64>()
                    .ok()
                    .map(Duration::from_secs)
            });
        }
        return Err(anyhow::Error::new(error).context(SafeFailure { delay: wait }));
    }
    Ok(())
}

pub fn backoff(attempt: usize, jitter_ms: u64, server: Duration) -> Duration {
    let base = Duration::from_millis((1_u64 << attempt.min(2)) * 1000 + jitter_ms.min(999));
    base.max(server)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_policy_and_permanent_errors() {
        for status in [408, 429, 500, 502, 503, 504] {
            assert!(transient_status(status));
        }
        for status in [400, 401, 403, 404, 407, 422, 501, 505] {
            assert!(!transient_status(status));
        }
        assert_eq!(delay(&anyhow::anyhow!("invalid YAML or whitelist")), None);
        assert_eq!(
            delay(&anyhow::Error::new(std::io::Error::from(
                std::io::ErrorKind::ConnectionReset
            ))),
            Some(Duration::ZERO)
        );
        assert_eq!(backoff(1, 0, Duration::ZERO), Duration::from_secs(2));
        assert_eq!(backoff(2, 999, Duration::ZERO), Duration::from_millis(4999));
        assert_eq!(
            backoff(1, 0, Duration::from_secs(90)),
            Duration::from_secs(90)
        );
        assert!(ATTEMPT_TIMEOUT * ATTEMPTS as u32 + Duration::from_secs(8) < TOTAL_TIMEOUT);
        let safe = anyhow::Error::new(SafeFailure {
            delay: Some(Duration::ZERO),
        });
        assert_eq!(delay(&safe), Some(Duration::ZERO));
        assert!(!safe.to_string().contains("http://"));
    }
    #[tokio::test]
    async fn http_retry_after_and_redaction() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (status, retry_after, expected) in [
            (503, "7", Some(7)),
            (429, "999", Some(999)),
            (503, "Wed, 21 Oct 2030 07:28:00 GMT", None),
            (403, "1", None),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = [0; 4096];
                let _ = s.read(&mut buf).await.unwrap();
                s.write_all(format!("HTTP/1.1 {status} Failure\r\nRetry-After: {retry_after}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            });
            let r = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(format!("http://{addr}/?secret=private"))
                .send()
                .await
                .unwrap();
            let e = check_http(&r).unwrap_err();
            assert_eq!(delay(&e), expected.map(Duration::from_secs));
            assert!(e.downcast_ref::<reqwest::Error>().is_some());
            assert!(!e.to_string().contains("private"));
            server.await.unwrap();
        }
    }
}
