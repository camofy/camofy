use super::*;
use serde_json::Value;
#[test]
fn identity_bindings_are_ordered_local_and_support_multiple_sources_or_only_independent_profiles() {
    let first = store::Resource {
        id: Uuid::new_v4(),
        kind: "profile".into(),
        version: 1,
        data: json!({"type":"source","name":"one","content":"rules: ['DOMAIN,one.example,DIRECT']"}),
    };
    let second = store::Resource {
        id: Uuid::new_v4(),
        kind: "profile".into(),
        version: 1,
        data: json!({"type":"source","name":"two","content":"rules: ['DOMAIN,two.example,DIRECT']"}),
    };
    let standalone = store::Resource {
        id: Uuid::new_v4(),
        kind: "profile".into(),
        version: 1,
        data: json!({"type":"overlay","name":"rules","content":"prepend-rules: ['DOMAIN,first.example,DIRECT']"}),
    };
    let pending = store::Resource {
        id: Uuid::new_v4(),
        kind: "profile".into(),
        version: 1,
        data: json!({"type":"source","name":"pending"}),
    };
    let all = vec![
        first.clone(),
        second.clone(),
        standalone.clone(),
        pending.clone(),
    ];
    let identity = json!({"profiles":[{"profile_id":first.id,"enabled":true},{"profile_id":second.id,"enabled":true},{"profile_id":standalone.id,"enabled":true},{"profile_id":pending.id,"enabled":false}]});
    let (a, _) = store::render_bundle(&all, &identity, "https://cloud.example").unwrap();
    let yaml = camofy::engine::parse(a["router"]["content"].as_str().unwrap()).unwrap();
    assert_eq!(yaml["rules"].as_sequence().unwrap().len(), 4);
    assert_eq!(yaml["rules"][0], "DOMAIN,cloud.example,DIRECT");
    assert_eq!(yaml["rules"][1], "DOMAIN,first.example,DIRECT");
    let mut independent = identity.clone();
    independent["profiles"] = json!([{"profile_id":standalone.id,"enabled":true}]);
    assert!(store::render_bundle(&all, &independent, "https://cloud.example").is_ok());
    let mut other = identity.clone();
    other["profiles"][2]["enabled"] = json!(false);
    let (b, _) = store::render_bundle(&all, &other, "https://cloud.example").unwrap();
    assert_ne!(a["router"]["hash"], b["router"]["hash"]);
    assert_eq!(
        store::render_bundle(&all, &identity, "https://cloud.example")
            .unwrap()
            .0,
        a
    );
    other["profiles"][3]["enabled"] = json!(true);
    assert!(store::render_bundle(&all, &other, "https://cloud.example").is_err());
}

#[test]
fn bundle_reports_per_target_errors_without_losing_clash() {
    let source = Uuid::new_v4();
    let resources = vec![store::Resource {
        id: source,
        kind: "profile".into(),
        version: 1,
        data: json!({"type":"source","content":"proxies: [{name: a, type: wireguard, server: example.com, port: 443}]\nrules: ['MATCH,a']"}),
    }];
    let (artifacts, _) = store::render_bundle(
        &resources,
        &json!({"profiles":[{"profile_id":source,"enabled":true}],"selections":{}}),
        "https://cloud.example",
    )
    .unwrap();
    assert!(artifacts["clash"]["content"].is_string());
    assert!(artifacts["shadowrocket-nodes"]["error"].is_string());
}

/// Uses a disposable PostgreSQL supplied by the caller, never a developer's production DB.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to a disposable PostgreSQL"]
async fn cloud_end_to_end() {
    use axum::http::HeaderMap;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use futures_util::{SinkExt, StreamExt};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(15)
        .connect(&std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required"))
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&db).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let origin = format!("http://{address}");
    let app = App {
        db: db.clone(),
        vault: security::Vault::new(&STANDARD.encode([17; 32])).unwrap(),
        origin: origin.clone(),
        legacy_origins: vec!["https://legacy.example".into()],
        secure: false,
        registration: true,
        private_egress: true,
        workers: 2,
        topics: Default::default(),
        hash_slots: Arc::new(Semaphore::new(4)),
    };
    let server = tokio::spawn(
        axum::serve(
            listener,
            router(app.clone()).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .into_future(),
    );
    sync::listen(app.clone()).await;
    let ip = Uuid::new_v4().into_bytes();
    let client = reqwest::Client::builder()
        .no_proxy()
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::new(
            127, 1, ip[0], ip[1],
        )))
        .build()
        .unwrap();
    async fn register(client: &reqwest::Client, base: &str) -> (String, Uuid) {
        let r=client.post(format!("{base}/api/auth/register")).json(&json!({"email":format!("{}@example.test",Uuid::new_v4()),"password":"integration-test-password"})).send().await.unwrap();
        assert_eq!(r.status(), 200);
        let cookie = r.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let body: serde_json::Value = r.json().await.unwrap();
        (
            cookie,
            Uuid::parse_str(body["user_id"].as_str().unwrap()).unwrap(),
        )
    }
    async fn request(
        client: &reqwest::Client,
        base: &str,
        cookie: &str,
        method: &str,
        path: &str,
        body: serde_json::Value,
        status: u16,
    ) -> serde_json::Value {
        let r = client
            .request(method.parse().unwrap(), format!("{base}/api{path}"))
            .header("cookie", cookie)
            .header("origin", base)
            .json(&body)
            .send()
            .await
            .unwrap();
        let actual = r.status();
        let text = r.text().await.unwrap();
        assert_eq!(actual.as_u16(), status, "{method} {path}: {text}");
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)
    }
    let (alice, alice_id) = register(&client, &origin).await;
    let (bob, bob_id) = register(&client, &origin).await;
    let yaml=Arc::new(Mutex::new("proxies:\n- {name: mine, type: ss, server: example.com, port: 443, cipher: aes-256-gcm, password: secret}\nproxy-groups:\n- {name: route, type: select, proxies: [mine, DIRECT]}\nrules: ['MATCH,route']\n".to_string()));
    let hits = Arc::new(AtomicUsize::new(0));
    let userinfo = Arc::new(Mutex::new(
        "upload=100;download=200;total=10000;expire=2000000000".to_string(),
    ));
    let mock = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = mock.local_addr().unwrap();
    let proxy = tokio::spawn(
        axum::serve(
            mock,
            Router::new().fallback(get({
                let hits = hits.clone();
                let yaml = yaml.clone();
                let userinfo = userinfo.clone();
                move |h: HeaderMap| {
                    let hits = hits.clone();
                    let yaml = yaml.clone();
                    let userinfo = userinfo.clone();
                    async move {
                        assert_eq!(
                            h.get("proxy-authorization").unwrap(),
                            &format!("Basic {}", STANDARD.encode("user:pass"))
                        );
                        hits.fetch_add(1, Ordering::SeqCst);
                        let mut headers = HeaderMap::new();
                        let info = userinfo.lock().await.clone();
                        if !info.is_empty() {
                            headers.insert("subscription-userinfo", info.parse().unwrap());
                        }
                        (headers, yaml.lock().await.clone())
                    }
                }
            })),
        )
        .into_future(),
    );
    let p=request(&client,&origin,&alice,"POST","/resources",json!({"kind":"proxy","data":{"name":"Proxy A","url":format!("http://user:pass@{proxy_addr}")}}),200).await;
    assert!(p["data"].get("url").is_none());
    let create_source = json!({"kind":"profile","data":{"name":"Main","type":"source","url":"http://localhost:59999/config.yaml","proxy_id":p["id"],"auto_refresh":true,"interval_seconds":300}});
    request(
        &client,
        &origin,
        &bob,
        "POST",
        "/resources",
        create_source.clone(),
        400,
    )
    .await; // foreign proxy forbidden
    let src = request(
        &client,
        &origin,
        &alice,
        "POST",
        "/resources",
        create_source,
        200,
    )
    .await;
    let sid = Uuid::parse_str(src["id"].as_str().unwrap()).unwrap();
    request(
        &client,
        &origin,
        &alice,
        "GET",
        &format!("/profiles/{sid}/content"),
        json!(null),
        409,
    )
    .await;
    request(
        &client,
        &origin,
        &bob,
        "GET",
        &format!("/profiles/{sid}/content"),
        json!(null),
        404,
    )
    .await;
    request(
        &client,
        &origin,
        "",
        "GET",
        &format!("/profiles/{sid}/content"),
        json!(null),
        401,
    )
    .await;
    // Concurrent workers must not fetch the same due job twice. Target port has no server:
    // success proves proxy is actually used, not an accidental direct request.
    let (a, b) = tokio::join!(worker::once(&app), worker::once(&app));
    assert!(a.unwrap() ^ b.unwrap());
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let history = request(
        &client,
        &origin,
        &alice,
        "GET",
        &format!("/profiles/{sid}/history"),
        json!(null),
        200,
    )
    .await;
    assert_eq!(history["items"][0]["status"], "ok");
    assert_eq!(history["items"][0]["reason"], "initial");
    request(
        &client,
        &origin,
        &bob,
        "GET",
        &format!("/profiles/{sid}/history"),
        json!(null),
        404,
    )
    .await;
    let cached = request(
        &client,
        &origin,
        &alice,
        "GET",
        &format!("/profiles/{sid}/content"),
        json!(null),
        200,
    )
    .await;
    assert_eq!(
        cached["content"].as_str().unwrap(),
        yaml.lock().await.as_str()
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "preview must not trigger upstream requests"
    );
    let overlay=request(&client,&origin,&alice,"POST","/resources",json!({"kind":"profile","data":{"name":"Work","type":"overlay","content":"prepend-rules: ['DOMAIN,work.example,DIRECT']"}}),200).await;
    let bundle=request(&client,&origin,&alice,"POST","/resources",json!({"kind":"bundle","data":{"name":"Everywhere","profiles":[{"profile_id":src["id"],"enabled":true},{"profile_id":overlay["id"],"enabled":true}],"selections":{"route":"DIRECT"}}}),200).await;
    assert!(bundle["data"]["published_revision"].is_string(), "{bundle}");
    let bid = bundle["id"].as_str().unwrap();
    let revision = bundle["data"]["published_revision"].clone();
    oauth_roundtrip(&app, &client, &origin, &alice, &bob, bid).await;
    let bobs = request(
        &client,
        &origin,
        &bob,
        "GET",
        "/resources",
        json!(null),
        200,
    )
    .await;
    assert_eq!(bobs.as_array().unwrap().len(), 0);
    request(
        &client,
        &origin,
        &bob,
        "DELETE",
        &format!("/resources/{bid}"),
        json!(null),
        404,
    )
    .await;
    request(
        &client,
        &origin,
        &bob,
        "GET",
        &format!("/bundles/{bid}/preview/clash"),
        json!(null),
        404,
    )
    .await;
    request(
        &client,
        &origin,
        &alice,
        "PUT",
        &format!("/resources/{bid}"),
        json!({"kind":"bundle","version":0,"data":bundle["data"]}),
        409,
    )
    .await;
    let device = request(
        &client,
        &origin,
        &alice,
        "POST",
        "/resources",
        json!({"kind":"device","data":{"name":"Router","bundle_id":bid}}),
        200,
    )
    .await;
    let token = request(
        &client,
        &origin,
        &alice,
        "POST",
        "/tokens",
        json!({"bundle_id":bid,"device_id":device["id"],"label":"router"}),
        200,
    )
    .await;
    let secret = token["token"].as_str().unwrap();
    let sub = format!("{origin}/sub/{secret}/clash");
    let r = client.get(&sub).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let etag = r.headers()["etag"].clone();
    assert_eq!(
        r.headers()["subscription-userinfo"],
        "upload=100; download=200; total=10000; expire=2000000000"
    );
    assert_eq!(r.headers()["profile-update-interval"], "1");
    let head = client.head(&sub).send().await.unwrap();
    assert_eq!(
        head.headers()["subscription-userinfo"],
        r.headers()["subscription-userinfo"]
    );
    assert!(head.text().await.unwrap().is_empty());
    let rendered = r.text().await.unwrap();
    assert!(rendered.contains("DOMAIN,work.example,DIRECT"));
    assert!(!rendered.contains("external-controller"));
    assert_eq!(
        client
            .get(&sub)
            .header("if-none-match", etag)
            .send()
            .await
            .unwrap()
            .status(),
        304
    );
    for format in ["router", "shadowrocket", "shadowrocket-nodes"] {
        assert_eq!(
            client
                .get(format!("{origin}/sub/{secret}/{format}"))
                .send()
                .await
                .unwrap()
                .status(),
            200,
            "{format}"
        );
    }
    let desired: serde_json::Value = client
        .get(format!("{origin}/api/sync/desired"))
        .bearer_auth(secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(desired["revision"], revision);
    // Optionally exercise a real Agent process against this API, using only a local fake core.
    let mut managed = None;
    if let Ok(binary) = std::env::var("CAMOFY_TEST_AGENT") {
        let root = std::env::temp_dir().join(format!("camofy-managed-test-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let port = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let number = port.local_addr().unwrap().port();
        drop(port);
        let settings = root.join("agent.json");
        tokio::fs::write(&settings, serde_json::to_vec(&json!({"subscription_url":format!("{origin}/sub/{secret}"),"mihomo":std::env::var("CAMOFY_TEST_CORE").unwrap(),"data_dir":root,"controller_port":number,"web_listen":null})).unwrap()).await.unwrap();
        let process = tokio::process::Command::new(binary)
            .arg(settings)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        managed = Some((process, root));
        await_report(
            &app,
            alice_id,
            device["id"].as_str().unwrap(),
            "revision",
            &revision,
        )
        .await;
    }
    // WebSocket notification is routed by tenant, and a new connection asks for resync.
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{address}/api/sync/ws"))
        .await
        .unwrap();
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"token":secret}).to_string().into(),
    ))
    .await
    .unwrap();
    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(msg.is_text());
    // Quota-only updates change public ETags, not config revisions or agent hashes.
    let before = client
        .get(format!("{origin}/api/sync/desired"))
        .bearer_auth(secret)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let old_etag = client.get(&sub).send().await.unwrap().headers()["etag"].clone();
    *userinfo.lock().await = "upload=150;download=300;total=10000;expire=2000000000".into();
    request(
        &client,
        &origin,
        &alice,
        "POST",
        &format!("/profiles/{sid}/refresh"),
        json!(null),
        202,
    )
    .await;
    assert!(worker::once(&app).await.unwrap());
    let after = client
        .get(format!("{origin}/api/sync/desired"))
        .bearer_auth(secret)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(before["revision"], after["revision"]);
    assert_eq!(before["hash"], after["hash"]);
    let changed = client
        .get(&sub)
        .header("if-none-match", old_etag)
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), 200);
    assert_eq!(
        changed.headers()["subscription-userinfo"],
        "upload=150; download=300; total=10000; expire=2000000000"
    );
    assert_eq!(changed.text().await.unwrap(), rendered);
    let summary = request(
        &client,
        &origin,
        &alice,
        "GET",
        &format!("/bundles/{bid}/usage"),
        json!(null),
        200,
    )
    .await;
    assert_eq!(summary["upload"], "150");
    assert_eq!(summary["known_pools"], 1);
    let previous_etag = client.get(&sub).send().await.unwrap().headers()["etag"].clone();
    *userinfo.lock().await = String::new();
    request(
        &client,
        &origin,
        &alice,
        "POST",
        &format!("/profiles/{sid}/refresh"),
        json!(null),
        202,
    )
    .await;
    assert!(worker::once(&app).await.unwrap());
    let missing = client
        .get(&sub)
        .header("if-none-match", previous_etag)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 200);
    assert!(missing.headers().get("subscription-userinfo").is_none());
    *userinfo.lock().await = "upload=150;download=300;total=10000;expire=2000000000".into();
    // Client writes cannot forge quota; pool-only edits do not fetch or republish.
    let records = request(
        &client,
        &origin,
        &alice,
        "GET",
        "/resources",
        json!(null),
        200,
    )
    .await;
    let item = records
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == src["id"])
        .unwrap();
    let mut data = item["data"].clone();
    data["usage"] = json!({"status":"ok","sample":{"total":999999}});
    data["usage_pool"] = json!("main-plan");
    request(
        &client,
        &origin,
        &alice,
        "PUT",
        &format!("/resources/{sid}"),
        json!({"kind":"profile","version":item["version"],"data":data}),
        200,
    )
    .await;
    assert!(!worker::once(&app).await.unwrap());
    let unaltered = client
        .get(format!("{origin}/api/sync/desired"))
        .bearer_auth(secret)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(before["revision"], unaltered["revision"]);
    assert!(
        client
            .get(&sub)
            .send()
            .await
            .unwrap()
            .headers()
            .get("subscription-userinfo")
            .is_none()
    );
    request(
        &client,
        &origin,
        &bob,
        "GET",
        &format!("/bundles/{bid}/usage"),
        json!(null),
        404,
    )
    .await;
    // Force the same scheduled job due (no user refresh endpoint) to verify scheduled proxy use.
    *yaml.lock().await = "not: [valid".into();
    sqlx::query("UPDATE fetch_jobs SET next_run=now() WHERE profile_id=$1")
        .bind(sid)
        .execute(&db)
        .await
        .unwrap();
    assert!(worker::once(&app).await.unwrap());
    assert_eq!(hits.load(Ordering::SeqCst), 4);
    let mut conn = db.acquire().await.unwrap();
    let stored = store::get(&app, &mut conn, alice_id, sid).await.unwrap();
    assert_eq!(stored.data["fetch_status"], "error");
    assert!(stored.data["content"].as_str().unwrap().contains("mine"));
    drop(conn);
    let content = client.get(&sub).send().await.unwrap().text().await.unwrap();
    assert_eq!(content, rendered);
    // A valid change publishes a new immutable revision and notification.
    *yaml.lock().await="proxies:\n- {name: mine, type: ss, server: example.org, port: 443, cipher: aes-256-gcm, password: changed}\nproxy-groups:\n- {name: route, type: select, proxies: [mine, DIRECT]}\nrules: ['MATCH,route']".into();
    request(
        &client,
        &origin,
        &alice,
        "POST",
        &format!("/profiles/{sid}/refresh"),
        json!(null),
        202,
    )
    .await;
    assert!(worker::once(&app).await.unwrap());
    assert_eq!(hits.load(Ordering::SeqCst), 5);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let msg = tokio::time::timeout_at(deadline, ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if msg.is_text() {
            break;
        }
        if let tokio_tungstenite::tungstenite::Message::Ping(p) = msg {
            ws.send(tokio_tungstenite::tungstenite::Message::Pong(p))
                .await
                .unwrap();
        }
    }
    let newer: serde_json::Value = client
        .get(format!("{origin}/api/sync/desired"))
        .bearer_auth(secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_ne!(newer["revision"], revision);
    if managed.is_some() {
        await_report(
            &app,
            alice_id,
            device["id"].as_str().unwrap(),
            "revision",
            &newer["revision"],
        )
        .await;
        request(
            &client,
            &origin,
            &alice,
            "POST",
            &format!("/devices/{}/test", device["id"].as_str().unwrap()),
            json!(null),
            200,
        )
        .await;
        await_report(
            &app,
            alice_id,
            device["id"].as_str().unwrap(),
            "delays",
            &json!({"mine":42}),
        )
        .await;
        for (action, state) in [("stop", "stopped"), ("restart", "running")] {
            let command = request(
                &client,
                &origin,
                &alice,
                "POST",
                &format!("/devices/{}/control", device["id"].as_str().unwrap()),
                json!({"action":action}),
                200,
            )
            .await;
            await_report(
                &app,
                alice_id,
                device["id"].as_str().unwrap(),
                "command_id",
                &command["id"],
            )
            .await;
            await_report(
                &app,
                alice_id,
                device["id"].as_str().unwrap(),
                "core_state",
                &json!(state),
            )
            .await;
        }
    }
    request(
        &client,
        &origin,
        &alice,
        "POST",
        &format!("/bundles/{bid}/rollback"),
        json!({"revision":revision}),
        204,
    )
    .await;
    assert_eq!(
        client.get(&sub).send().await.unwrap().text().await.unwrap(),
        rendered
    );
    let report=client.post(format!("{origin}/api/sync/report")).bearer_auth(secret).json(&json!({"revision":revision,"status":"applied","message":"ok","logs":"this must never be stored"})).send().await.unwrap();
    assert_eq!(report.status(), 204);
    let records = request(
        &client,
        &origin,
        &alice,
        "GET",
        "/resources",
        json!(null),
        200,
    )
    .await;
    assert!(!records.to_string().contains("this must never be stored"));
    let sealed: serde_json::Value = sqlx::query_scalar("SELECT data FROM resources WHERE id=$1")
        .bind(sid)
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(sealed["sealed"].is_string());
    assert!(!sealed.to_string().contains("example"));
    // Cookie-authenticated writes need same-origin CSRF protection.
    for (request_origin, expected) in [
        ("https://legacy.example", 200),
        ("https://evil.example", 403),
    ] {
        let response = client
            .post(format!("{origin}/api/tokens"))
            .header("cookie", &alice)
            .header("origin", request_origin)
            .json(&json!({"bundle_id":bid,"label":"origin migration test"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), expected);
        if expected == 200 {
            let value: serde_json::Value = response.json().await.unwrap();
            assert!(
                value["subscription_base"]
                    .as_str()
                    .unwrap()
                    .starts_with(&origin)
            );
        }
    }
    assert_eq!(
        client
            .post(format!("{origin}/api/tokens"))
            .header("cookie", &alice)
            .json(&json!({"bundle_id":bid,"label":"bad"}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    request(
        &client,
        &origin,
        &alice,
        "DELETE",
        &format!("/tokens/{}", camofy::digest(secret)),
        json!(null),
        204,
    )
    .await;
    assert_eq!(client.get(&sub).send().await.unwrap().status(), 401);
    assert_eq!(
        client
            .get(format!("{origin}/api/sync/desired"))
            .bearer_auth(secret)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    ws.close(None).await.unwrap();
    if let Some((mut process, root)) = managed {
        process.kill().await.unwrap();
        // A force-killed parent cannot run destructors; explicitly stop its test-only child.
        let running = camofy::engine::parse(
            &tokio::fs::read_to_string(root.join("running.yaml"))
                .await
                .unwrap(),
        )
        .unwrap();
        let _ = client
            .post(format!(
                "http://{}/fixture/stop",
                running["external-controller"].as_str().unwrap()
            ))
            .bearer_auth(running["secret"].as_str().unwrap())
            .send()
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
    // Identity URL remains stable as a local binding is toggled; another identity is unaffected.
    let only = request(&client,&origin,&alice,"POST","/resources",json!({"kind":"profile","data":{"name":"Independent","type":"overlay","content":"rules: ['DOMAIN,independent.example,DIRECT']"}}),200).await;
    let bindings = json!([{"profile_id":only["id"],"enabled":true},{"profile_id":overlay["id"],"enabled":false}]);
    let own = request(
        &client,
        &origin,
        &alice,
        "POST",
        "/resources",
        json!({"kind":"bundle","data":{"name":"Independent identity","profiles":bindings}}),
        200,
    )
    .await;
    let peer = request(
        &client,
        &origin,
        &alice,
        "POST",
        "/resources",
        json!({"kind":"bundle","data":{"name":"Peer identity","profiles":bindings}}),
        200,
    )
    .await;
    let url = own["data"]["subscription_url"].as_str().unwrap();
    assert!(!url.ends_with("/router"));
    let neutral = client.get(url).send().await.unwrap();
    assert_eq!(neutral.status(), 200);
    let etag = neutral.headers()["etag"].clone();
    let revision_header = neutral.headers()["x-camofy-revision"].clone();
    let neutral_yaml = neutral.text().await.unwrap();
    let legacy = client.get(format!("{url}/router")).send().await.unwrap();
    assert_eq!(legacy.headers()["etag"], etag);
    assert_eq!(legacy.headers()["x-camofy-revision"], revision_header);
    assert_eq!(legacy.text().await.unwrap(), neutral_yaml);
    assert_eq!(
        client
            .get(url)
            .header("if-none-match", etag)
            .send()
            .await
            .unwrap()
            .status(),
        304
    );
    let first_yaml = client.get(url).send().await.unwrap().text().await.unwrap();
    let canonical = url
        .split("/sub/")
        .nth(1)
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    assert_eq!(
        client
            .get(format!("{origin}/api/sync/desired"))
            .bearer_auth(canonical)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let mut edits = own["data"].clone();
    edits["profiles"][1]["enabled"] = json!(true);
    let changed = request(
        &client,
        &origin,
        &alice,
        "PUT",
        &format!("/resources/{}", own["id"].as_str().unwrap()),
        json!({"kind":"bundle","version":own["version"],"data":edits}),
        200,
    )
    .await;
    assert_eq!(
        changed["data"]["subscription_url"],
        own["data"]["subscription_url"]
    );
    assert_ne!(
        changed["data"]["published_revision"],
        own["data"]["published_revision"]
    );
    assert_ne!(
        client.get(url).send().await.unwrap().text().await.unwrap(),
        first_yaml
    );
    assert_eq!(
        client
            .get(peer["data"]["subscription_url"].as_str().unwrap())
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        first_yaml
    );
    // Disabled associations still prevent deletion and cannot reference another tenant.
    request(
        &client,
        &origin,
        &alice,
        "DELETE",
        &format!("/resources/{}", overlay["id"].as_str().unwrap()),
        json!(null),
        409,
    )
    .await;
    request(
        &client,
        &origin,
        &bob,
        "POST",
        "/resources",
        json!({"kind":"bundle","data":{"name":"Foreign","profiles":bindings}}),
        400,
    )
    .await;
    request(&client,&origin,&alice,"POST","/resources",json!({"kind":"profile","data":{"name":"Wrong activation","type":"overlay","content":"{}","enabled":true}}),400).await;
    request(
        &client,
        &origin,
        &alice,
        "DELETE",
        &format!("/tokens/{}", camofy::digest(canonical)),
        json!(null),
        204,
    )
    .await;
    assert_eq!(client.get(url).send().await.unwrap().status(), 401);
    // New supplier credentials require an authenticated preview. A forged provisioning
    // status must not bypass it, and foreign resources stay inaccessible before any API call.
    let xiequ = json!({"name":"Short-lived", "provider":"xiequ", "whitelist_uid":"123", "whitelist_key":"secret",
        "extract_url":"http://api.xiequ.cn/VAD/GetIp.aspx?act=get&num=1&uid=123&vkey=secret", "whitelist_ip":"8.8.8.8"});
    request(
        &client,
        &origin,
        &alice,
        "POST",
        "/resources",
        json!({"kind":"proxy","data":xiequ}),
        400,
    )
    .await;
    request(
        &client,
        &origin,
        &bob,
        "PUT",
        &format!("/resources/{}", p["id"].as_str().unwrap()),
        json!({"kind":"proxy","version":1,"data":xiequ}),
        404,
    )
    .await;
    // A manual refresh arriving during an active lease must not get swallowed.
    let hold = yaml.lock().await;
    let prior_hits = hits.load(Ordering::SeqCst);
    request(
        &client,
        &origin,
        &alice,
        "POST",
        &format!("/profiles/{sid}/refresh"),
        json!(null),
        202,
    )
    .await;
    let running = tokio::spawn({
        let app = app.clone();
        async move { worker::once(&app).await }
    });
    for _ in 0..100 {
        if hits.load(Ordering::SeqCst) > prior_hits {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(hits.load(Ordering::SeqCst), prior_hits + 1);
    request(
        &client,
        &origin,
        &alice,
        "POST",
        &format!("/profiles/{sid}/refresh"),
        json!(null),
        202,
    )
    .await;
    drop(hold);
    assert!(running.await.unwrap().unwrap());
    assert!(worker::once(&app).await.unwrap());
    assert_eq!(hits.load(Ordering::SeqCst), prior_hits + 2);
    // A provider error never silently selects direct egress or erases the cached YAML.
    let mut conn = db.acquire().await.unwrap();
    let mut pr = store::get(
        &app,
        &mut conn,
        alice_id,
        Uuid::parse_str(p["id"].as_str().unwrap()).unwrap(),
    )
    .await
    .unwrap();
    pr.data = xiequ;
    pr.data["whitelist_ip"] = serde_json::Value::Null;
    store::put(&app, &mut conn, alice_id, &pr).await.unwrap();
    let cached = store::get(&app, &mut conn, alice_id, sid)
        .await
        .unwrap()
        .data["content"]
        .clone();
    drop(conn);
    let previous_hits = hits.load(Ordering::SeqCst);
    request(
        &client,
        &origin,
        &alice,
        "POST",
        &format!("/profiles/{sid}/refresh"),
        json!(null),
        202,
    )
    .await;
    assert!(worker::once(&app).await.unwrap());
    let mut conn = db.acquire().await.unwrap();
    let failed = store::get(&app, &mut conn, alice_id, sid).await.unwrap();
    assert_eq!(failed.data["content"], cached);
    assert_eq!(failed.data["fetch_status"], "error");
    assert!(failed.data["error"].as_str().unwrap().contains("白名单"));
    assert_eq!(hits.load(Ordering::SeqCst), previous_hits);
    drop(conn);
    let history = request(
        &client,
        &origin,
        &alice,
        "GET",
        &format!("/profiles/{sid}/history"),
        json!(null),
        200,
    )
    .await;
    assert_eq!(history["items"][0]["error_code"], "proxy_failure");
    assert!(
        history["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["reason"] == "scheduled" && h["error_code"] == "invalid_yaml")
    );
    assert!(!history.to_string().contains("user:pass"));
    assert!(!history.to_string().contains("http://"));
    sqlx::query("INSERT INTO refresh_history(user_id,profile_id,claim,reason,status) SELECT $1,$2,gen_random_uuid(),'scheduled','ok' FROM generate_series(1,110)").bind(alice_id).bind(sid).execute(&db).await.unwrap();
    let mut conn = db.acquire().await.unwrap();
    crate::history::prune(&mut conn, alice_id, sid)
        .await
        .unwrap();
    drop(conn);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM refresh_history WHERE profile_id=$1")
        .bind(sid)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 100);
    let page = request(
        &client,
        &origin,
        &alice,
        "GET",
        &format!("/profiles/{sid}/history"),
        json!(null),
        200,
    )
    .await;
    assert_eq!(page["items"].as_array().unwrap().len(), 20);
    let cursor = page["next_cursor"].as_str().unwrap();
    let next = request(
        &client,
        &origin,
        &alice,
        "GET",
        &format!("/profiles/{sid}/history?before={cursor}"),
        json!(null),
        200,
    )
    .await;
    assert!(!page["items"].as_array().unwrap().iter().any(|p| {
        next["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| p["id"] == n["id"])
    }));
    // Primary rotation never revokes historical links or device authorization.
    let link_profile = request(&client,&origin,&alice,"POST","/resources",json!({"kind":"profile","data":{"name":"Lifecycle base","type":"overlay","content":"rules: ['MATCH,DIRECT']"}}),200).await;
    let identity = request(
        &client,
        &origin,
        &alice,
        "POST",
        "/resources",
        json!({"kind":"bundle","data":{"name":"Link lifecycle","profiles":[{"profile_id":link_profile["id"],"enabled":true}]}}),
        200,
    )
    .await;
    let iid = identity["id"].as_str().unwrap();
    let old_url = identity["data"]["subscription_url"].as_str().unwrap();
    let old_hash = camofy::digest(old_url.rsplit('/').next().unwrap());
    let path = format!("/bundles/{iid}/subscription-links");
    let historical = request(
        &client,
        &origin,
        &alice,
        "POST",
        "/tokens",
        json!({"bundle_id":iid,"label":"legacy"}),
        200,
    )
    .await;
    let history_hash = camofy::digest(historical["token"].as_str().unwrap());
    let history_url = historical["subscription_base"].as_str().unwrap();
    let dev = request(
        &client,
        &origin,
        &alice,
        "POST",
        "/resources",
        json!({"kind":"device","data":{"name":"Lifecycle device","bundle_id":iid}}),
        200,
    )
    .await;
    let dev_id = dev["id"].as_str().unwrap();
    let dev_token = request(
        &client,
        &origin,
        &alice,
        "POST",
        "/tokens",
        json!({"bundle_id":iid,"device_id":dev_id,"label":"device"}),
        200,
    )
    .await;
    let dev_secret = dev_token["token"].as_str().unwrap();
    let other = request(
        &client,
        &origin,
        &alice,
        "POST",
        "/resources",
        json!({"kind":"bundle","data":{"name":"Unrelated identity","profiles":[{"profile_id":link_profile["id"],"enabled":true}]}}),
        200,
    )
    .await;
    let list = request(&client, &origin, &alice, "GET", &path, json!(null), 200).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], history_hash);
    assert!(list[0]["created_at"].is_i64());
    request(&client, &origin, &bob, "GET", &path, json!(null), 404).await;
    request(
        &client,
        &origin,
        &bob,
        "POST",
        &format!("{path}/reset"),
        json!({"version":identity["version"]}),
        404,
    )
    .await;
    request(
        &client,
        &origin,
        &bob,
        "DELETE",
        &format!("{path}/{history_hash}"),
        json!(null),
        404,
    )
    .await;
    request(
        &client,
        &origin,
        &alice,
        "DELETE",
        &format!("{path}/{old_hash}"),
        json!(null),
        409,
    )
    .await;
    request(
        &client,
        &origin,
        &alice,
        "DELETE",
        &format!("{path}/{}", camofy::digest(dev_secret)),
        json!(null),
        404,
    )
    .await;
    assert_eq!(
        client
            .post(format!("{origin}/api{path}/reset"))
            .header("cookie", &alice)
            .json(&json!({"version":identity["version"]}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let reset_path = format!("{path}/reset");
    let (first, second) = tokio::join!(
        client
            .post(format!("{origin}/api{reset_path}"))
            .header("cookie", &alice)
            .header("origin", &origin)
            .json(&json!({"version":identity["version"]}))
            .send(),
        client
            .post(format!("{origin}/api{reset_path}"))
            .header("cookie", &alice)
            .header("origin", &origin)
            .json(&json!({"version":identity["version"]}))
            .send()
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    let rotated: Value = if first.status() == 200 {
        assert_eq!(second.status(), 409);
        first.json().await.unwrap()
    } else {
        assert_eq!(first.status(), 409);
        assert_eq!(second.status(), 200);
        second.json().await.unwrap()
    };
    assert_eq!(
        rotated["data"]["published_revision"],
        identity["data"]["published_revision"]
    );
    let new_url = rotated["data"]["subscription_url"].as_str().unwrap();
    assert_ne!(new_url, old_url);
    for url in [old_url.to_string(), format!("{old_url}/clash")] {
        assert_eq!(client.get(url).send().await.unwrap().status(), 401);
    }
    for url in [
        new_url,
        history_url,
        other["data"]["subscription_url"].as_str().unwrap(),
    ] {
        assert_eq!(client.get(url).send().await.unwrap().status(), 200);
    }
    assert_eq!(
        client
            .get(format!("{origin}/api/sync/desired"))
            .bearer_auth(dev_secret)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    request(
        &client,
        &origin,
        &alice,
        "DELETE",
        &format!(
            "/bundles/{}/subscription-links/{history_hash}",
            other["id"].as_str().unwrap()
        ),
        json!(null),
        404,
    )
    .await;
    request(
        &client,
        &origin,
        &alice,
        "DELETE",
        &format!("{path}/{history_hash}"),
        json!(null),
        204,
    )
    .await;
    assert_eq!(client.get(history_url).send().await.unwrap().status(), 401);
    assert_eq!(client.get(new_url).send().await.unwrap().status(), 200);
    request(
        &client,
        &origin,
        &alice,
        "DELETE",
        &format!("/resources/{dev_id}"),
        json!(null),
        204,
    )
    .await;
    assert_eq!(
        client
            .get(format!("{origin}/api/sync/desired"))
            .bearer_auth(dev_secret)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(client.get(new_url).send().await.unwrap().status(), 200);
    assert!(
        request(&client, &origin, &alice, "GET", &path, json!(null), 200)
            .await
            .as_array()
            .unwrap()
            .is_empty()
    );
    server.abort();
    proxy.abort();
    sqlx::query("DELETE FROM users WHERE id=ANY($1)")
        .bind(vec![alice_id, bob_id])
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
}

async fn oauth_roundtrip(
    app: &App,
    client: &reqwest::Client,
    base: &str,
    alice: &str,
    bob: &str,
    bundle: &str,
) {
    async fn start(client: &reqwest::Client, base: &str) -> Value {
        let r=client.post(format!("{base}/api/oauth/device_authorization")).form(&json!({"client_id":"camofy-agent","scope":"device:sync","device_name":"OAuth test","return_uri":"http://192.168.1.1:3000/bind/complete","state":"a".repeat(64)})).send().await.unwrap();
        assert_eq!(r.status(), 200);
        r.json().await.unwrap()
    }
    async fn exchange(client: &reqwest::Client, base: &str, code: &str) -> (u16, Value) {
        let r=client.post(format!("{base}/api/oauth/token")).form(&json!({"client_id":"camofy-agent","grant_type":"urn:ietf:params:oauth:grant-type:device_code","device_code":code})).send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap())
    }
    let flow = start(client, base).await;
    let device = flow["device_code"].as_str().unwrap();
    let user_code = flow["user_code"].as_str().unwrap();
    assert!(
        !flow["verification_uri_complete"]
            .as_str()
            .unwrap()
            .contains(device)
    );
    assert_eq!(
        exchange(client, base, device).await.1["error"],
        "authorization_pending"
    );
    assert_eq!(exchange(client, base, device).await.1["error"], "slow_down");
    let data = json!({"user_code":user_code,"approve":true,"bundle_id":bundle});
    let unauthorized = client
        .post(format!("{base}/api/oauth/approve"))
        .json(&data)
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), 403);
    let csrf = client
        .post(format!("{base}/api/oauth/approve"))
        .header("cookie", alice)
        .header("origin", "https://evil.example")
        .json(&data)
        .send()
        .await
        .unwrap();
    assert_eq!(csrf.status(), 403);
    let foreign = client
        .post(format!("{base}/api/oauth/approve"))
        .header("cookie", bob)
        .header("origin", base)
        .json(&data)
        .send()
        .await
        .unwrap();
    assert_eq!(foreign.status(), 404);
    let approved = client
        .post(format!("{base}/api/oauth/approve"))
        .header("cookie", alice)
        .header("origin", base)
        .json(&data)
        .send()
        .await
        .unwrap();
    assert_eq!(approved.status(), 200);
    let callback: Value = approved.json().await.unwrap();
    let url = url::Url::parse(callback["return_uri"].as_str().unwrap()).unwrap();
    assert_eq!(url.query_pairs().count(), 1);
    assert_eq!(url.query_pairs().next().unwrap().0, "state");
    let duplicate = client
        .post(format!("{base}/api/oauth/approve"))
        .header("cookie", alice)
        .header("origin", base)
        .json(&data)
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate.status(), 400);
    sqlx::query(
        "UPDATE device_authorizations SET next_poll=now()-interval '1 second' WHERE device_hash=$1",
    )
    .bind(camofy::digest(device))
    .execute(&app.db)
    .await
    .unwrap();
    let (a, b) = tokio::join!(
        exchange(client, base, device),
        exchange(client, base, device)
    );
    assert_eq!(
        usize::from(a.0 == 200) + usize::from(b.0 == 200),
        1,
        "one-time grant must be consumed atomically"
    );
    let token = if a.0 == 200 { a.1 } else { b.1 };
    assert_eq!(token["scope"], "device:sync");
    let access = token["access_token"].as_str().unwrap();
    assert!(
        token.get("subscription_url").is_none(),
        "device authorization must not issue a user-configured subscription URL"
    );
    let mut conn = app.db.acquire().await.unwrap();
    let owner: Uuid = sqlx::query_scalar("SELECT user_id FROM access_tokens WHERE hash=$1")
        .bind(camofy::digest(access))
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    let original = store::get(
        app,
        &mut conn,
        owner,
        Uuid::parse_str(token["device_id"].as_str().unwrap()).unwrap(),
    )
    .await
    .unwrap();
    let identity = store::get(
        app,
        &mut conn,
        owner,
        Uuid::parse_str(original.data["bundle_id"].as_str().unwrap()).unwrap(),
    )
    .await
    .unwrap();
    drop(conn);
    let second: Value = client
        .post(format!("{base}/api/resources"))
        .header("cookie", alice)
        .header("origin", base)
        .json(&json!({"kind":"bundle","data":identity.data}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let changed = client.put(format!("{base}/api/resources/{}",original.id)).header("cookie",alice).header("origin",base).json(&json!({"kind":"device","version":original.version,"data":{"name":"reassigned","bundle_id":second["id"]}})).send().await.unwrap();
    assert_eq!(changed.status(), 200);
    let desired: Value = client
        .get(format!("{base}/api/sync/desired"))
        .bearer_auth(access)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        desired["revision"], second["data"]["published_revision"],
        "same device credential follows reassignment"
    );
    assert_eq!(
        client
            .post(format!("{base}/api/devices/{}/control", original.id))
            .header("cookie", alice)
            .header("origin", "https://evil.example")
            .json(&json!({"action":"stop"}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .get(format!("{base}/api/resources"))
            .bearer_auth(access)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!("{base}/api/sync/desired"))
            .bearer_auth(access)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        exchange(client, base, device).await.1["error"],
        "expired_token"
    );
    let revoke = client
        .delete(format!("{base}/api/tokens/{}", camofy::digest(access)))
        .header("cookie", alice)
        .header("origin", base)
        .send()
        .await
        .unwrap();
    assert_eq!(revoke.status(), 204);
    assert_eq!(
        client
            .get(format!("{base}/api/sync/desired"))
            .bearer_auth(access)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let denied = start(client, base).await;
    let r = client
        .post(format!("{base}/api/oauth/approve"))
        .header("cookie", alice)
        .header("origin", base)
        .json(&json!({"user_code":denied["user_code"],"approve":false}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        exchange(client, base, denied["device_code"].as_str().unwrap())
            .await
            .1["error"],
        "access_denied"
    );
    let expired = start(client, base).await;
    sqlx::query("UPDATE device_authorizations SET expires_at=now()-interval '1 second' WHERE device_hash=$1").bind(camofy::digest(expired["device_code"].as_str().unwrap())).execute(&app.db).await.unwrap();
    assert_eq!(
        exchange(client, base, expired["device_code"].as_str().unwrap())
            .await
            .1["error"],
        "expired_token"
    );
}

async fn await_report(app: &App, user: Uuid, device: &str, key: &str, value: &serde_json::Value) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let mut conn = app.db.acquire().await.unwrap();
        let record = store::get(app, &mut conn, user, Uuid::parse_str(device).unwrap())
            .await
            .unwrap();
        if record.data["reported"][key] == *value {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "device {key} did not converge: {}",
            record.data
        );
        drop(conn);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
