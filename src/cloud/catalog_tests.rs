use super::*;
use axum::http::StatusCode;

fn publication(slug: &str, version: &str, extra: bool) -> Value {
    let mut rules =
        vec![json!({"kind":"DOMAIN-SUFFIX","value":"video.example","no_resolve":false})];
    if extra {
        rules.push(json!({"kind":"DOMAIN","value":"new.example","no_resolve":false}));
    }
    json!({"slug":slug,"version":version,"manifest":{"name":"Video","summary":"Test rules","category":"Tests","notes":"Inline test data","default_policy":null,"sources":[{"url":"https://example.com/rules","revision":"a".repeat(40),"sha256":"b".repeat(64),"license":"MIT","license_text":"Test fixture permission notice","attribution":"Test author"}]},"rules":rules})
}

#[test]
fn strict_rule_parameters_and_dependency_checks() {
    for rule in [
        json!({"kind":"DOMAIN","value":"example.com,DIRECT"}),
        json!({"kind":"SCRIPT","value":"evil.example"}),
        json!({"kind":"IP-CIDR","value":"127.0.0.1/33"}),
        json!({"kind":"IP-CIDR6","value":"127.0.0.1/32"}),
        json!({"kind":"DOMAIN","value":"example.com","no_resolve":true}),
    ] {
        assert!(validate_rules(&[serde_json::from_value(rule).unwrap()]).is_err());
    }
    let mut p = Resource {
        id: Uuid::new_v4(),
        kind: "profile".into(),
        version: 1,
        data: json!({"store":{},"_package":publication("test","1.0.0",false)}),
    };
    assert!(compile(&p, &json!({})).is_err());
    assert!(compile(&p, &json!({"parameters":{"policy":"DIRECT\nMATCH,REJECT"}})).is_err());
    assert!(compile(&p, &json!({"parameters":{"policy":"DIRECT","script":"x"}})).is_err());
    let yaml = compile(&p, &json!({"parameters":{"policy":"DIRECT"}})).unwrap();
    assert!(yaml.contains("DOMAIN-SUFFIX,video.example,DIRECT"));
    p.data["_package"]["manifest"]["default_policy"] = json!("REJECT");
    assert!(compile(&p, &json!({})).unwrap().contains("REJECT"));
    assert!(
        camofy::engine::compose_profiles(
            &["rules: ['RULE-SET,missing,DIRECT']".into()],
            &Default::default()
        )
        .is_err()
    );
    assert_eq!(
        diagnostics("rules: ['DOMAIN,one.example,DIRECT', 'DOMAIN,one.example,REJECT']").len(),
        1
    );
    assert!(
        validate_manifest(
            &serde_json::from_value(publication("x", "1.0.0", false)["manifest"].clone()).unwrap()
        )
        .is_ok()
    );
}

#[test]
fn provenance_cannot_inject_yaml_through_comment_line_separators() {
    let p = Resource {
        id: Uuid::new_v4(),
        kind: "profile".into(),
        version: 1,
        data: json!({"type":"overlay","content":"rules: ['MATCH,DIRECT']","provenance":[{"license_text":"MIT\rmode: global\u{85}mixed-port: 22\u{2028}rules: []\u{2029}secret: injected"}]}),
    };
    let (a, _) = crate::store::render_bundle(
        &[p.clone()],
        &json!({"profiles":[{"profile_id":p.id,"enabled":true}]}),
        "https://cloud.example",
    )
    .unwrap();
    let v = camofy::engine::parse(a["clash"]["content"].as_str().unwrap()).unwrap();
    assert_eq!(v["mode"], "rule");
    assert!(v["secret"].is_null());
    assert_eq!(v["rules"][0], "DOMAIN,cloud.example,DIRECT");
}

/// Isolated database only; real HTTP/auth/SQL/compiler, no external network or subscriber traffic.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to disposable PostgreSQL"]
async fn catalog_end_to_end() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::{future::IntoFuture, sync::Arc};
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&db).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = App {
        db: db.clone(),
        vault: crate::security::Vault::new(&STANDARD.encode([31; 32])).unwrap(),
        origin: origin.clone(),
        legacy_origins: vec![],
        secure: false,
        registration: true,
        private_egress: false,
        workers: 1,
        captcha: None,
        topics: Default::default(),
        hash_slots: Arc::new(tokio::sync::Semaphore::new(2)),
    };
    let server = tokio::spawn(
        axum::serve(
            listener,
            crate::router(app).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .into_future(),
    );
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    async fn req(
        c: &reqwest::Client,
        base: &str,
        cookie: &str,
        method: &str,
        path: &str,
        v: Value,
        status: u16,
    ) -> Value {
        let r = c
            .request(method.parse().unwrap(), format!("{base}/api{path}"))
            .header("cookie", cookie)
            .header("origin", base)
            .json(&v)
            .send()
            .await
            .unwrap();
        let s = r.status();
        let text = r.text().await.unwrap();
        assert_eq!(s.as_u16(), status, "{method} {path}: {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }
    let mut users = vec![];
    for _ in 0..2 {
        let r=client.post(format!("{origin}/api/auth/register")).json(&json!({"email":format!("{}@example.test",Uuid::new_v4()),"password":"test-only-long-password","nickname":"Camofy"})).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let cookie = r.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let u: Value = r.json().await.unwrap();
        users.push((
            cookie,
            Uuid::parse_str(u["user_id"].as_str().unwrap()).unwrap(),
        ));
    }
    let (alice, uid) = &users[0];
    let (bob, _) = &users[1];
    let slug = format!("test-{}", Uuid::new_v4().simple());
    let a = req(
        &client,
        &origin,
        alice,
        "PATCH",
        "/account",
        json!({"nickname":"Publisher"}),
        200,
    )
    .await;
    assert_eq!(a["nickname"], "Publisher");
    req(
        &client,
        &origin,
        alice,
        "PATCH",
        "/account",
        json!({"nickname":"X","email":"change@example.test"}),
        422,
    )
    .await;
    req(
        &client,
        &origin,
        alice,
        "PATCH",
        "/account",
        json!({"nickname":"  "}),
        400,
    )
    .await;
    req(
        &client,
        &origin,
        bob,
        "POST",
        "/store/packages",
        publication(&slug, "1.0.0", false),
        403,
    )
    .await;
    sqlx::query("INSERT INTO catalog_publishers(user_id,display_name) VALUES($1,'Publisher')")
        .bind(uid)
        .execute(&db)
        .await
        .unwrap();
    let p1 = req(
        &client,
        &origin,
        alice,
        "POST",
        "/store/packages",
        publication(&slug, "1.0.0", false),
        200,
    )
    .await;
    let repeated = req(
        &client,
        &origin,
        alice,
        "POST",
        "/store/packages",
        publication(&slug, "1.0.0", false),
        200,
    )
    .await;
    assert_eq!(p1, repeated);
    req(
        &client,
        &origin,
        alice,
        "POST",
        "/store/packages",
        publication(&slug, "1.0.0", true),
        409,
    )
    .await;
    let p2 = req(
        &client,
        &origin,
        alice,
        "POST",
        "/store/packages",
        publication(&slug, "1.1.0", true),
        200,
    )
    .await;
    req(
        &client,
        &origin,
        alice,
        "PATCH",
        "/account",
        json!({"nickname":"Camofy"}),
        200,
    )
    .await;
    let detail = req(
        &client,
        &origin,
        bob,
        "GET",
        &format!("/store/packages/{slug}"),
        Value::Null,
        200,
    )
    .await;
    assert_eq!(detail["publisher"], "Camofy");
    assert!(detail.get("owner_id").is_none());
    assert!(!detail.to_string().contains("@example.test"));
    let pid = Uuid::new_v4();
    let installation = json!({"version_id":p1["id"],"profile_id":pid});
    let profile = req(
        &client,
        &origin,
        bob,
        "POST",
        "/store/install",
        installation.clone(),
        200,
    )
    .await;
    assert_eq!(profile["data"]["origin"], "store");
    let source_path = format!("/profiles/{pid}/source-preview");
    let source = req(
        &client,
        &origin,
        bob,
        "POST",
        &source_path,
        json!({"version":1,"policy":"DIRECT"}),
        200,
    )
    .await;
    assert!(
        source["content"]
            .as_str()
            .unwrap()
            .contains("prepend-rules:")
    );
    assert!(
        source["content"]
            .as_str()
            .unwrap()
            .contains("video.example,DIRECT")
    );
    assert!(source["content"].as_str().unwrap().contains("MIT"));
    assert_eq!(source["parameterized"], false);
    let parameterized = req(
        &client,
        &origin,
        bob,
        "POST",
        &source_path,
        json!({"version":1}),
        200,
    )
    .await;
    assert_eq!(parameterized["parameterized"], true);
    req(
        &client,
        &origin,
        alice,
        "POST",
        &source_path,
        json!({"version":1}),
        404,
    )
    .await;
    req(
        &client,
        &origin,
        bob,
        "POST",
        &source_path,
        json!({"version":2}),
        409,
    )
    .await;
    req(
        &client,
        &origin,
        bob,
        "POST",
        &source_path,
        json!({"version":1,"policy":"DIRECT,REJECT"}),
        400,
    )
    .await;
    req(
        &client,
        &origin,
        bob,
        "POST",
        "/store/install",
        installation.clone(),
        200,
    )
    .await;
    req(
        &client,
        &origin,
        alice,
        "POST",
        "/store/install",
        installation,
        409,
    )
    .await;
    let raw: Value = sqlx::query_scalar("SELECT data FROM resources WHERE id=$1")
        .bind(pid)
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(!raw.to_string().contains("_package"));
    req(&client,&origin,bob,"POST","/resources",json!({"kind":"profile","data":{"name":"fake","type":"overlay","origin":"store","store":{"version_id":p1["id"]}}}),400).await;
    let base = req(&client,&origin,bob,"POST","/resources",json!({"kind":"profile","data":{"name":"Base","type":"overlay","content":"proxies: [{name: test, type: ss, server: example.com, port: 443, cipher: aes-256-gcm, password: sample-only}]\nproxy-groups: [{name: Route, type: select, proxies: [test, DIRECT]}]\nrules: ['MATCH,Route']\n"}}),200).await;
    let data = json!({"name":"Test identity","profiles":[{"profile_id":base["id"],"enabled":true},{"profile_id":pid,"enabled":true,"parameters":{"policy":"DIRECT"}}]});
    let preview = req(
        &client,
        &origin,
        bob,
        "POST",
        "/store/identity-preview",
        json!({"data":data}),
        200,
    )
    .await;
    assert!(
        preview["artifacts"]["clash"]["content"]
            .as_str()
            .unwrap()
            .contains("video.example,DIRECT")
    );
    assert!(preview["artifacts"]["shadowrocket"]["content"].is_string());
    let bundle = req(
        &client,
        &origin,
        bob,
        "POST",
        "/resources",
        json!({"kind":"bundle","data":data}),
        200,
    )
    .await;
    let bid = bundle["id"].as_str().unwrap();
    let mut invalid = data.clone();
    invalid["profiles"][1]["parameters"]["policy"] = json!("MissingGroup");
    req(
        &client,
        &origin,
        bob,
        "POST",
        "/resources",
        json!({"kind":"bundle","data":invalid}),
        400,
    )
    .await;
    let mut second = data.clone();
    second["profiles"][1]["parameters"]["policy"] = json!("REJECT");
    let second = req(
        &client,
        &origin,
        bob,
        "POST",
        "/resources",
        json!({"kind":"bundle","data":second}),
        200,
    )
    .await;
    let rejected = req(
        &client,
        &origin,
        bob,
        "GET",
        &format!("/bundles/{}/preview/clash", second["id"].as_str().unwrap()),
        Value::Null,
        200,
    )
    .await;
    assert!(
        rejected["content"]
            .as_str()
            .unwrap()
            .contains("video.example,REJECT")
    );
    let up = json!({"version":1,"version_id":p2["id"]});
    req(
        &client,
        &origin,
        alice,
        "POST",
        &format!("/profiles/{pid}/upgrade-preview"),
        up.clone(),
        404,
    )
    .await;
    req(
        &client,
        &origin,
        bob,
        "POST",
        &format!("/profiles/{pid}/upgrade"),
        up.clone(),
        409,
    )
    .await;
    let preview = req(
        &client,
        &origin,
        bob,
        "POST",
        &format!("/profiles/{pid}/upgrade-preview"),
        up.clone(),
        200,
    )
    .await;
    assert_eq!(preview["affected"].as_array().unwrap().len(), 2);
    assert_eq!(preview["added"], 1);
    let mut up = up;
    up["preview_digest"] = preview["preview_digest"].clone();
    req(
        &client,
        &origin,
        bob,
        "POST",
        &format!("/profiles/{pid}/upgrade"),
        up.clone(),
        200,
    )
    .await;
    req(
        &client,
        &origin,
        bob,
        "POST",
        &format!("/profiles/{pid}/upgrade"),
        up,
        409,
    )
    .await;
    let upgraded = req(
        &client,
        &origin,
        bob,
        "GET",
        &format!("/bundles/{bid}/preview/clash"),
        Value::Null,
        200,
    )
    .await;
    assert!(
        upgraded["content"]
            .as_str()
            .unwrap()
            .contains("new.example,DIRECT")
    );
    let rollback = json!({"version":2,"version_id":p1["id"]});
    let preview = req(
        &client,
        &origin,
        bob,
        "POST",
        &format!("/profiles/{pid}/upgrade-preview"),
        rollback.clone(),
        200,
    )
    .await;
    let mut rollback = rollback;
    rollback["preview_digest"] = preview["preview_digest"].clone();
    req(
        &client,
        &origin,
        bob,
        "POST",
        &format!("/profiles/{pid}/upgrade"),
        rollback,
        200,
    )
    .await;
    let restored = req(
        &client,
        &origin,
        bob,
        "GET",
        &format!("/bundles/{bid}/preview/clash"),
        Value::Null,
        200,
    )
    .await;
    assert!(
        !restored["content"]
            .as_str()
            .unwrap()
            .contains("new.example")
    );
    let fork = req(
        &client,
        &origin,
        bob,
        "POST",
        &format!("/profiles/{pid}/fork"),
        json!({"version":3,"policy":"DIRECT"}),
        200,
    )
    .await;
    assert!(fork["data"].get("store").is_none());
    assert!(fork["data"]["provenance"].is_array());
    let lock: Value = sqlx::query_scalar(
        "SELECT catalog_lock FROM revisions WHERE bundle_id=$1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(Uuid::parse_str(bid).unwrap())
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(lock[0]["version_id"], p1["id"]);
    assert!(lock[0]["artifact_hash"].is_string());
    for (_, id) in &users {
        sqlx::query("DELETE FROM users WHERE id=$1")
            .bind(id)
            .execute(&db)
            .await
            .unwrap();
    }
    server.abort();
}
