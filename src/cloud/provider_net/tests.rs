use super::*;
use axum::{Router, http::StatusCode, response::IntoResponse, routing::get};
#[test]
fn transport_context_records_destination_without_query_credentials() {
    let target: url::Url = "https://api.example.test/GetIp.aspx?uid=SECRET&vkey=SECRET"
        .parse()
        .unwrap();
    let addresses = ["8.8.8.8:443".parse().unwrap()];
    let context = transport_context(&target, &addresses, "connect_or_headers");
    assert_eq!(context.host, "api.example.test");
    assert_eq!(context.port, 443);
    assert_eq!(context.addresses, &addresses);
    assert_eq!(context.phase, "connect_or_headers");
    assert!(!format!("{context:?}").contains("SECRET"));
}

#[tokio::test]
async fn transport_preserves_safe_categories_and_retry_after_without_leaking_credentials() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/ok", get(|| async { "one-result" }))
                .route(
                    "/limited",
                    get(|| async {
                        (
                            StatusCode::TOO_MANY_REQUESTS,
                            [("Retry-After", "7")],
                            "SECRET",
                        )
                    }),
                )
                .route(
                    "/forbidden",
                    get(|| async { (StatusCode::FORBIDDEN, "SECRET") }),
                )
                .route(
                    "/redirect",
                    get(|| async {
                        (
                            StatusCode::FOUND,
                            [("Location", "http://127.0.0.1:1/SECRET")],
                            "SECRET",
                        )
                            .into_response()
                    }),
                )
                .route("/large", get(|| async { "x".repeat(65537) }))
                .route(
                    "/slow",
                    get(|| async {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        "SECRET"
                    }),
                ),
        )
        .await
        .unwrap();
    });
    for (path, kind, delay) in [
        (
            "limited",
            FailureKind::Http(429),
            Some(Duration::from_secs(7)),
        ),
        ("forbidden", FailureKind::Http(403), None),
        ("redirect", FailureKind::Http(302), None),
        ("large", FailureKind::Response, None),
    ] {
        let target = format!("http://{address}/{path}?key=SECRET")
            .parse()
            .unwrap();
        let error = get_text(&target, None, true).await.unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(error.delay, delay);
        let error = anyhow::Error::new(error);
        assert_eq!(retry::delay(&error), delay);
        let (code, message) = crate::history::failure(&error, "proxy");
        assert!(code.starts_with("proxy_extract_"));
        assert!(!format!("{error:#} {error:?} {message}").contains("SECRET"));
        assert!(!message.contains("http://"));
    }
    let slow = format!("http://{address}/slow?key=SECRET").parse().unwrap();
    let error = get_at(&slow, vec![address], true, Duration::from_millis(100))
        .await
        .unwrap_err();
    assert_eq!(error.kind, FailureKind::Timeout);
    assert_eq!(error.diagnostic().0, "proxy_extract_timeout");
    let ok = format!("http://{address}/ok").parse().unwrap();
    assert_eq!(get_text(&ok, None, true).await.unwrap(), "one-result");
    assert_eq!(
        get_text(&ok, None, false).await.unwrap_err().kind,
        FailureKind::DnsInvalid
    );
    assert_eq!(
        get_at(
            &ok,
            vec!["127.0.0.1:1".parse().unwrap()],
            true,
            Duration::from_secs(1)
        )
        .await
        .unwrap_err()
        .kind,
        FailureKind::DnsInvalid
    );
    for url in [
        "ftp://example.com",
        "https://user:SECRET@example.com",
        "https://example.com/#SECRET",
    ] {
        assert_eq!(
            get_text(&url.parse().unwrap(), None, false)
                .await
                .unwrap_err()
                .kind,
            FailureKind::Configuration
        );
    }
    server.abort();
}

#[tokio::test]
async fn refused_connection_is_not_reported_as_dns_or_subscription_failure() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let error = get_text(
        &format!("http://{address}/?key=SECRET").parse().unwrap(),
        None,
        true,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, FailureKind::Connect);
    assert_eq!(error.diagnostic().0, "proxy_extract_connect");
    assert_eq!(error.delay, Some(Duration::ZERO));
}

#[test]
fn diagnostics_distinguish_dns_and_http_but_contain_no_supplier_identity() {
    for (kind, code) in [
        (FailureKind::DnsTimeout, "proxy_dns_timeout"),
        (FailureKind::DnsUnavailable, "proxy_dns_failure"),
        (FailureKind::DnsInvalid, "proxy_dns_invalid"),
        (FailureKind::Connect, "proxy_extract_connect"),
        (FailureKind::Timeout, "proxy_extract_timeout"),
        (FailureKind::Http(503), "proxy_extract_http"),
    ] {
        let error = anyhow::Error::new(Failure::permanent(kind));
        let (actual, message) = crate::history::failure(&error, "proxy");
        assert_eq!(actual, code);
        for forbidden in ["xiequ", "api.", "http://", "https://"] {
            assert!(!message.contains(forbidden));
        }
    }
}
