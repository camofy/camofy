//! HTTP contracts: identity scope, publication, revision policy and cache variants.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to disposable PostgreSQL"]
async fn compatibility_http_end_to_end() {
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&db).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = App {
        db: db.clone(),
        vault: security::Vault::new(&STANDARD.encode([31; 32])).unwrap(),
        origin: origin.clone(),
        legacy_origins: vec![],
        secure: false,
        registration: false,
        private_egress: true,
        workers: 1,
        captcha: None,
        topics: Default::default(),
        hash_slots: Arc::new(Semaphore::new(1)),
    };
    let server = tokio::spawn(
        axum::serve(
            listener,
            router(app.clone()).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .into_future(),
    );
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut users = vec![];
    let mut sessions = vec![];
    for _ in 0..2 {
        let id = Uuid::new_v4();
        let session = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO users(id,email,password) VALUES($1,$2,'unused')")
            .bind(id)
            .bind(format!("{id}@example.test"))
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO sessions(hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 hour')",
        )
        .bind(camofy::digest(&session))
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
        users.push(id);
        sessions.push(session);
    }
    async fn call(
        client: &reqwest::Client,
        origin: &str,
        session: &str,
        method: &str,
        path: &str,
        body: Value,
        expected: u16,
    ) -> Value {
        let response = client
            .request(method.parse().unwrap(), format!("{origin}/api{path}"))
            .bearer_auth(session)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert_eq!(status.as_u16(), expected, "{method} {path}: {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }
    assert_eq!(
        client
            .get(format!("{origin}/api/client-compatibility"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let matrix = call(
        &client,
        &origin,
        &sessions[0],
        "GET",
        "/client-compatibility",
        json!(null),
        200,
    )
    .await;
    assert!(matrix["clients"].as_array().unwrap().len() >= 15);
    let source = call(&client,&origin,&sessions[0],"POST","/resources",json!({"kind":"profile","data":{"name":"Fixture","type":"overlay","content":"proxies: [{name: Classic, type: ss, server: proxy.example, port: 443, cipher: aes-256-gcm, password: sample}, {name: Mieru, type: mieru, server: proxy.example, port: 443, username: sample, password: sample, transport: TCP}, {name: AnyTLS, type: anytls, server: proxy.example, port: 443, password: sample}]\nproxy-groups: [{name: Choose, type: select, proxies: [Mieru, AnyTLS, Classic]}]\nrules: ['MATCH,Choose']"}}),200).await;
    let data =
        json!({"name":"Fixture identity","profiles":[{"profile_id":source["id"],"enabled":true}]});
    let bundle = call(
        &client,
        &origin,
        &sessions[0],
        "POST",
        "/resources",
        json!({"kind":"bundle","data":data}),
        200,
    )
    .await;
    let other = call(
        &client,
        &origin,
        &sessions[0],
        "POST",
        "/resources",
        json!({"kind":"bundle","data":data}),
        200,
    )
    .await;
    let id = bundle["id"].as_str().unwrap();
    let sub = bundle["data"]["subscription_url"].as_str().unwrap();
    let old = "ClashMetaForAndroid/2.10.2.Meta";
    let new = "mihomo/1.19.17";
    let response = client
        .get(sub)
        .header("User-Agent", old)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["vary"], "User-Agent");
    assert_eq!(response.headers()["x-camofy-filtered"], "2");
    let old_etag = response.headers()["etag"].clone();
    let content = response.text().await.unwrap();
    assert!(!content.contains("type: mieru"));
    assert!(!content.contains("type: anytls"));
    assert!(content.contains("type: ss"));
    let conditional = client
        .get(sub)
        .header("User-Agent", old)
        .header("If-None-Match", &old_etag)
        .send()
        .await
        .unwrap();
    assert_eq!(conditional.status(), 304);
    assert_eq!(conditional.headers()["vary"], "User-Agent");
    let head = client
        .head(sub)
        .header("User-Agent", old)
        .send()
        .await
        .unwrap();
    assert_eq!(head.headers()["etag"], old_etag);
    assert!(head.bytes().await.unwrap().is_empty());
    for ua in [new, "clash.meta", "Unrecognized/1.0"] {
        let response = client
            .get(sub)
            .header("User-Agent", ua)
            .header("If-None-Match", &old_etag)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["x-camofy-filtered"], "0");
        assert!(response.text().await.unwrap().contains("type: mieru"));
    }
    assert!(
        client
            .get(sub)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
            .contains("type: anytls")
    );
    let preview_path = format!("/bundles/{id}/compatibility-preview");
    call(
        &client,
        &origin,
        &sessions[1],
        "POST",
        &preview_path,
        json!({"user_agent":old}),
        404,
    )
    .await;
    call(
        &client,
        &origin,
        &sessions[0],
        "POST",
        &preview_path,
        json!({"user_agent":"x".repeat(513)}),
        400,
    )
    .await;
    let preview = call(
        &client,
        &origin,
        &sessions[0],
        "POST",
        &preview_path,
        json!({"user_agent":old}),
        200,
    )
    .await;
    assert_eq!(preview["report"]["removed"], 2);
    assert_eq!(preview["report"]["client"]["version"], "2.10.2");
    // Device synchronization keeps the immutable content/hash regardless of UA.
    let token = sub.rsplit('/').next().unwrap();
    let manifest = client
        .get(format!("{origin}/api/sync/desired"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let revision = manifest["revision"].as_str().unwrap();
    let download = client
        .get(format!("{origin}/api/sync/revisions/{revision}/router"))
        .bearer_auth(token)
        .header("User-Agent", old)
        .send()
        .await
        .unwrap();
    assert_eq!(download.status(), 200);
    let raw = download.bytes().await.unwrap();
    assert_eq!(camofy::digest(&raw), manifest["hash"].as_str().unwrap());
    let mut manual = data.clone();
    manual["node_filter"] = json!({"auto":false,"exclude_types":["anytls"]});
    let mut edited = call(
        &client,
        &origin,
        &sessions[0],
        "PUT",
        &format!("/resources/{id}"),
        json!({"kind":"bundle","version":bundle["version"],"data":manual}),
        200,
    )
    .await;
    assert_ne!(
        edited["data"]["published_revision"],
        bundle["data"]["published_revision"]
    );
    let response = client
        .get(sub)
        .header("User-Agent", old)
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["x-camofy-filtered"], "1");
    let content = response.text().await.unwrap();
    assert!(content.contains("type: mieru"));
    assert!(!content.contains("type: anytls"));
    assert_eq!(
        client
            .get(other["data"]["subscription_url"].as_str().unwrap())
            .header("User-Agent", old)
            .send()
            .await
            .unwrap()
            .headers()["x-camofy-filtered"],
        "2"
    );
    let manual_revision = edited["data"]["published_revision"].clone();
    manual["node_filter"]["auto"] = json!(true);
    edited = call(
        &client,
        &origin,
        &sessions[0],
        "PUT",
        &format!("/resources/{id}"),
        json!({"kind":"bundle","version":edited["version"],"data":manual}),
        200,
    )
    .await;
    assert_ne!(edited["data"]["published_revision"], manual_revision);
    assert_eq!(
        client
            .get(sub)
            .header("User-Agent", old)
            .send()
            .await
            .unwrap()
            .headers()["x-camofy-filtered"],
        "2"
    );
    manual["node_filter"]["exclude_types"] = json!(["ss", "mieru", "anytls"]);
    call(
        &client,
        &origin,
        &sessions[0],
        "PUT",
        &format!("/resources/{id}"),
        json!({"kind":"bundle","version":edited["version"],"data":manual}),
        400,
    )
    .await;
    let unchanged = call(
        &client,
        &origin,
        &sessions[0],
        "GET",
        "/resources",
        json!(null),
        200,
    )
    .await;
    assert_eq!(
        unchanged
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap()["version"],
        edited["version"]
    );
    call(
        &client,
        &origin,
        &sessions[0],
        "POST",
        &format!("/bundles/{id}/rollback"),
        json!({"revision":bundle["data"]["published_revision"]}),
        204,
    )
    .await;
    assert_eq!(
        client
            .get(sub)
            .header("User-Agent", old)
            .send()
            .await
            .unwrap()
            .headers()["x-camofy-filtered"],
        "2"
    );
    server.abort();
    for user in users {
        sqlx::query("DELETE FROM users WHERE id=$1")
            .bind(user)
            .execute(&db)
            .await
            .unwrap();
    }
}
