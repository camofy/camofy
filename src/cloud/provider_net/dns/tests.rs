use super::*;
use axum::{Router, body::Bytes, routing::post};
use hickory_proto::rr::{
    Record,
    rdata::{A, CNAME},
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn answer(query: &Message, ip: Ipv4Addr, ttl: u32) -> Message {
    let mut response = query.clone();
    response.metadata.message_type = MessageType::Response;
    response.add_answer(Record::from_rdata(
        query.queries[0].name().clone(),
        ttl,
        RData::A(A(ip)),
    ));
    response
}

#[test]
fn validates_question_chain_public_addresses_and_ttl() {
    let q = query("supplier.example", DnsPolicy::default()).unwrap();
    let good = answer(&q, Ipv4Addr::new(8, 8, 8, 8), 600);
    let parsed = parse(&good.to_vec().unwrap(), &q).unwrap();
    assert_eq!(parsed.ips, ["8.8.8.8".parse::<IpAddr>().unwrap()]);
    assert!(parsed.expires <= Instant::now() + MAX_TTL);
    assert!(parsed.expires > Instant::now() + Duration::from_secs(299));
    for ip in [
        Ipv4Addr::LOCALHOST,
        Ipv4Addr::new(10, 0, 0, 1),
        Ipv4Addr::new(169, 254, 169, 254),
    ] {
        let bad = answer(&q, ip, 60);
        assert!(parse(&bad.to_vec().unwrap(), &q).is_err());
        let mut mixed = good.clone();
        mixed.add_answer(Record::from_rdata(
            q.queries[0].name().clone(),
            60,
            RData::A(A(ip)),
        ));
        assert!(parse(&mixed.to_vec().unwrap(), &q).is_err());
    }
    let mut unrelated = good.clone();
    unrelated.add_answer(Record::from_rdata(
        Name::from_ascii("unrelated.example").unwrap(),
        0,
        RData::A(A(Ipv4Addr::LOCALHOST)),
    ));
    assert!(parse(&unrelated.to_vec().unwrap(), &q).is_ok());
    let mut cname = q.clone();
    let terminal = Name::from_ascii("cdn.example").unwrap();
    cname.metadata.message_type = MessageType::Response;
    cname
        .add_answer(Record::from_rdata(
            q.queries[0].name().clone(),
            1,
            RData::CNAME(CNAME(terminal.clone())),
        ))
        .add_answer(Record::from_rdata(
            terminal.clone(),
            300,
            RData::A(A(Ipv4Addr::new(8, 8, 4, 4))),
        ));
    let parsed = parse(&cname.to_vec().unwrap(), &q).unwrap();
    assert!(parsed.expires <= Instant::now() + Duration::from_secs(1));
    cname.add_answer(Record::from_rdata(
        terminal,
        60,
        RData::CNAME(CNAME(q.queries[0].name().clone())),
    ));
    assert!(parse(&cname.to_vec().unwrap(), &q).is_err());
    let zero = answer(&q, Ipv4Addr::new(8, 8, 8, 8), 0);
    assert!(parse(&zero.to_vec().unwrap(), &q).unwrap().expires <= Instant::now());
    for bad in [
        {
            let mut m = good.clone();
            m.metadata.id = 42;
            m
        },
        {
            let mut m = good.clone();
            m.metadata.truncation = true;
            m
        },
        {
            let mut m = good.clone();
            m.metadata.message_type = MessageType::Query;
            m
        },
        {
            let mut m = good.clone();
            m.metadata.response_code = ResponseCode::NXDomain;
            m
        },
        answer(
            &query("other.example", DnsPolicy::default()).unwrap(),
            Ipv4Addr::new(8, 8, 8, 8),
            60,
        ),
    ] {
        assert!(parse(&bad.to_vec().unwrap(), &q).is_err());
    }
    assert!(parse(b"malformed", &q).is_err());
    for status in [ResponseCode::ServFail, ResponseCode::Refused] {
        let mut failed = good.clone();
        failed.metadata.response_code = status;
        let error = parse(&failed.to_vec().unwrap(), &q).err().unwrap();
        assert_eq!(error.kind, FailureKind::DnsUnavailable);
        assert!(error.delay.is_some());
    }
    assert!(query("", DnsPolicy::default()).is_err());
    assert!(
        query(
            "supplier.example",
            DnsPolicy {
                subnet: Some((Ipv4Addr::LOCALHOST, 33))
            }
        )
        .is_err()
    );
}

#[test]
fn subnet_is_explicit_and_never_inherited_between_policies() {
    use hickory_proto::rr::rdata::opt::EdnsCode;
    let china = DnsPolicy {
        subnet: Some((Ipv4Addr::new(223, 5, 5, 0), 24)),
    };
    for policy in [DnsPolicy::default(), china] {
        let message = query("supplier.example", policy).unwrap();
        let expected = policy.subnet.unwrap_or((Ipv4Addr::UNSPECIFIED, 0));
        assert_eq!(
            message
                .edns
                .as_ref()
                .unwrap()
                .options()
                .get(EdnsCode::Subnet),
            Some(&EdnsOption::Subnet(ClientSubnet::new(
                expected.0.into(),
                expected.1,
                0
            )))
        );
    }
}

async fn serve(
    ip: Ipv4Addr,
    delay: Duration,
) -> (url::Url, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let count = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/dns-query", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let router = Router::new().fallback(post({
        let count = count.clone();
        move |body: Bytes| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                assert!(!String::from_utf8_lossy(&body).contains("SECRET"));
                let q = Message::from_vec(&body).unwrap();
                tokio::time::sleep(delay).await;
                (
                    [("Content-Type", "application/dns-message")],
                    answer(&q, ip, 60).to_vec().unwrap(),
                )
            }
        }
    }));
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (url, count, server)
}

fn local(endpoints: Vec<url::Url>) -> Resolver {
    Resolver {
        endpoints: endpoints.into_iter().map(|u| ("test", u)).collect(),
        cache: Mutex::new(HashMap::new()),
        timeout: Duration::from_secs(2),
        private: true,
    }
}

#[tokio::test]
async fn races_first_valid_not_first_response_and_coalesces_cache_misses() {
    let (bad, bad_count, bad_server) = serve(Ipv4Addr::LOCALHOST, Duration::ZERO).await;
    let (good, count, server) = serve(Ipv4Addr::new(8, 8, 8, 8), Duration::from_millis(30)).await;
    let resolver = local(vec![bad, good]);
    let target = "https://supplier.example/extract?key=SECRET"
        .parse()
        .unwrap();
    let policy = DnsPolicy::default();
    let mut requests = (0..8)
        .map(|_| resolver.resolve(&target, policy))
        .collect::<FuturesUnordered<_>>();
    while let Some(answer) = requests.next().await {
        assert_eq!(
            answer.unwrap(),
            ["8.8.8.8:443".parse::<SocketAddr>().unwrap()]
        );
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(bad_count.load(Ordering::SeqCst), 1);
    // Scheme/port/query are not DNS inputs; a cached IP is rebound to the requested port.
    let other_port = "http://supplier.example:8080/else?token=SECRET"
        .parse()
        .unwrap();
    assert_eq!(
        resolver.resolve(&other_port, policy).await.unwrap()[0].port(),
        8080
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let china = DnsPolicy {
        subnet: Some((Ipv4Addr::new(223, 5, 5, 0), 24)),
    };
    resolver.resolve(&target, china).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
    resolver
        .resolve(
            &"https://different-supplier.example".parse().unwrap(),
            policy,
        )
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 3);
    let slot = resolver
        .cache
        .lock()
        .await
        .get(&Resolver::key(&target, policy))
        .unwrap()
        .clone();
    slot.lock().await.as_mut().unwrap().expires = Instant::now();
    resolver.resolve(&target, policy).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 4);
    resolver.invalidate(&target, policy).await;
    resolver.resolve(&target, policy).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 5);
    server.abort();
    bad_server.abort();
}

#[tokio::test]
async fn slow_resolver_does_not_delay_valid_answer_and_all_failures_are_bounded() {
    let (slow, _, slow_server) = serve(Ipv4Addr::new(8, 8, 8, 8), Duration::from_secs(5)).await;
    let (good, _, server) = serve(Ipv4Addr::new(8, 8, 4, 4), Duration::ZERO).await;
    let resolver = local(vec![slow.clone(), good]);
    let target = "https://supplier.example?key=SECRET".parse().unwrap();
    tokio::time::timeout(
        Duration::from_secs(1),
        resolver.resolve(&target, DnsPolicy::default()),
    )
    .await
    .unwrap()
    .unwrap();
    let mut resolver = local(vec![slow]);
    resolver.timeout = Duration::from_millis(100);
    let error = resolver
        .resolve(&target, DnsPolicy::default())
        .await
        .unwrap_err();
    assert_eq!(error.kind, FailureKind::DnsTimeout);
    assert!(!format!("{error:?}").contains("SECRET"));
    server.abort();
    slow_server.abort();
}

#[tokio::test]
async fn cancelled_lookup_does_not_poison_singleflight_and_cache_stays_bounded() {
    let (slow, _, slow_server) = serve(Ipv4Addr::new(8, 8, 8, 8), Duration::from_secs(5)).await;
    let (good, _, server) = serve(Ipv4Addr::new(8, 8, 4, 4), Duration::ZERO).await;
    let mut resolver = Arc::new(local(vec![slow]));
    let target: url::Url = "https://supplier.example".parse().unwrap();
    let task = tokio::spawn({
        let resolver = resolver.clone();
        let target = target.clone();
        async move { resolver.resolve(&target, DnsPolicy::default()).await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    task.abort();
    let _ = task.await;
    Arc::get_mut(&mut resolver).unwrap().endpoints = vec![("test", good)];
    resolver
        .resolve(&target, DnsPolicy::default())
        .await
        .unwrap();
    {
        let mut cache = resolver.cache.lock().await;
        for n in 0..CACHE_LIMIT {
            cache.insert(
                Key {
                    host: format!("{n}.example"),
                    policy: DnsPolicy::default(),
                },
                Arc::new(Mutex::new(None)),
            );
        }
    }
    resolver
        .resolve(
            &"https://new.example".parse().unwrap(),
            DnsPolicy::default(),
        )
        .await
        .unwrap();
    assert!(resolver.cache.lock().await.len() <= CACHE_LIMIT);
    server.abort();
    slow_server.abort();
}

/// Explicit opt-in smoke test: sends only public DNS names, never supplier extraction requests.
#[tokio::test]
#[ignore = "requires outbound HTTPS to public DoH services; no supplier credentials needed"]
async fn public_doh_with_and_without_regional_policy() {
    let resolver = resolver();
    for (host, policy) in [
        ("example.com", DnsPolicy::default()),
        (
            "api.xiequ.cn",
            DnsPolicy {
                subnet: Some((Ipv4Addr::new(223, 5, 5, 0), 24)),
            },
        ),
    ] {
        let query = query(host, policy).unwrap();
        let mut pending = resolver
            .endpoints
            .iter()
            .map(|(label, endpoint)| {
                let query = &query;
                async move {
                    let start = Instant::now();
                    let result = tokio::time::timeout(
                        resolver.timeout,
                        resolver.ask("smoke", endpoint, query),
                    )
                    .await
                    .map_err(|_| Failure::transient(FailureKind::DnsTimeout))
                    .and_then(|r| r);
                    println!(
                        "{label}: host={host}, elapsed_ms={}, outcome={}",
                        start.elapsed().as_millis(),
                        result
                            .as_ref()
                            .err()
                            .map(|e| e.diagnostic().0)
                            .unwrap_or("ok")
                    );
                    result
                }
            })
            .collect::<FuturesUnordered<_>>();
        let mut successes = 0;
        while let Some(result) = pending.next().await {
            if let Ok(answer) = result {
                assert!(!answer.ips.is_empty());
                assert!(answer.ips.into_iter().all(security::public_ip));
                successes += 1;
            }
        }
        assert!(successes > 0, "neither DoH service resolved {host}");
    }
}
