use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

/// Runs only against a disposable local PostgreSQL selected by the caller.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to a disposable PostgreSQL"]
async fn outbound_contract_http_end_to_end() {
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(
            &std::env::var("TEST_DATABASE_URL").expect("disposable TEST_DATABASE_URL required"),
        )
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&db).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = App {
        db,
        vault: security::Vault::new(&STANDARD.encode([23; 32])).unwrap(),
        origin: origin.clone(),
        legacy_origins: vec![],
        secure: false,
        registration: true,
        private_egress: true,
        workers: 1,
        captcha: None,
        topics: Default::default(),
        hash_slots: Arc::new(Semaphore::new(2)),
    };
    let server = tokio::spawn(
        axum::serve(
            listener,
            router(app).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .into_future(),
    );
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let registration = client.post(format!("{origin}/api/auth/register"))
        .header("origin", &origin)
        .json(&json!({"email":format!("{}@example.test",Uuid::new_v4()),"password":"integration-test-password"}))
        .send().await.unwrap();
    assert_eq!(registration.status(), 200);
    let cookie = registration.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    async fn request(
        client: &reqwest::Client,
        origin: &str,
        cookie: &str,
        method: &str,
        path: &str,
        body: Value,
        expected: u16,
    ) -> Value {
        let response = client
            .request(method.parse().unwrap(), format!("{origin}/api{path}"))
            .header("origin", origin)
            .header("cookie", cookie)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let text = response.text().await.unwrap();
        assert_eq!(status, expected, "{method} {path}: {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }
    let provider = request(&client,&origin,&cookie,"POST","/resources",json!({"kind":"profile","data":{
        "type":"overlay","name":"Provider","content":"proxy-groups: [{name: Gateway, type: select, proxies: [DIRECT]}]",
        "exports":[{"key":"primary","label":"Primary","kind":"group","target":"Gateway"}]
    }}),200).await;
    let consumer = request(&client,&origin,&cookie,"POST","/resources",json!({"kind":"profile","data":{
        "type":"overlay","name":"Consumer","content":"prepend-proxies: [{name: Exit, type: socks5, server: example.org, port: 1080}]",
        "inputs":[{"key":"hop","label":"Hop","kind":"outbound","section":"prepend-proxies","name":"Exit","field":"dialer-proxy"}]
    }}),200).await;
    let identity_data = json!({"name":"Test identity","profiles":[
        {"profile_id":consumer["id"],"enabled":true},
        {"profile_id":provider["id"],"enabled":true}],
        "default_outbound":{"source":"export","profile_id":provider["id"],"key":"primary"}});
    let preview = request(
        &client,
        &origin,
        &cookie,
        "POST",
        "/store/identity-preview",
        json!({"data":identity_data}),
        200,
    )
    .await;
    assert_eq!(preview["capability_lock"][0]["resolved"], "Gateway");
    let identity = request(
        &client,
        &origin,
        &cookie,
        "POST",
        "/resources",
        json!({"kind":"bundle","data":identity_data}),
        200,
    )
    .await;
    let subscription = identity["data"]["subscription_url"].as_str().unwrap();
    let published = client
        .get(subscription)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(
        camofy::engine::parse(&published).unwrap()["proxies"][0]["dialer-proxy"],
        "Gateway"
    );
    let mut broken = provider["data"].clone();
    broken["exports"][0]["target"] = json!("Removed");
    request(
        &client,
        &origin,
        &cookie,
        "PUT",
        &format!("/resources/{}", provider["id"].as_str().unwrap()),
        json!({"kind":"profile","version":provider["version"],"data":broken}),
        200,
    )
    .await;
    let records = request(
        &client,
        &origin,
        &cookie,
        "GET",
        "/resources",
        Value::Null,
        200,
    )
    .await;
    let current = records
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == identity["id"])
        .unwrap();
    assert_eq!(
        current["data"]["published_revision"],
        identity["data"]["published_revision"]
    );
    assert!(
        current["data"]["error"]
            .as_str()
            .unwrap()
            .contains("不存在")
    );
    assert_eq!(
        client
            .get(subscription)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        published
    );
    let mut disabled = identity_data.clone();
    disabled["profiles"][1]["enabled"] = json!(false);
    request(
        &client,
        &origin,
        &cookie,
        "POST",
        "/store/identity-preview",
        json!({"data":disabled}),
        400,
    )
    .await;

    // New contracts are independent of YAML field names and legacy default outlets.
    let source = request(&client,&origin,&cookie,"POST","/resources",json!({"kind":"profile","data":{
        "type":"overlay","name":"Typed provider","content":"proxy-groups: [{name: Path, type: select, proxies: [DIRECT]}]",
        "provides":[{"key":"route","label":"My route","type":"outbound",
            "selector":{"source":"named","section":"proxy-groups","match_field":"name",
                "match_value":"Path","value_field":"name"}}]
    }}),200).await;
    let template = request(&client,&origin,&cookie,"POST","/resources",json!({"kind":"profile","data":{
        "type":"overlay","name":"Typed consumer",
        "content":"mixed-port: '{{camofy.listen}}'\nprepend-proxies: [{name: Home, type: socks5, server: example.org, port: 1080, dialer-proxy: '{{camofy.path}}'}]",
        "variables":[{"key":"listen","label":"Listen","type":"integer","required":true},
            {"key":"path","label":"Path","type":"outbound","required":true}]
    }}),200).await;
    let typed_identity = json!({"name":"Typed identity","profiles":[
        {"profile_id":template["id"],"enabled":true,"variable_bindings":{
            "listen":{"source":"literal","value":7897},
            "path":{"source":"identity","key":"main"}}},
        {"profile_id":source["id"],"enabled":true}],
        "identity_values":{"main":{"type":"outbound","binding":{
            "source":"export","profile_id":source["id"],"key":"route"}}}});
    let preview = request(
        &client,
        &origin,
        &cookie,
        "POST",
        "/store/identity-preview",
        json!({"data":typed_identity}),
        200,
    )
    .await;
    assert_eq!(preview["variable_resolutions"].as_array().unwrap().len(), 2);
    assert_eq!(preview["artifacts"]["router"]["error"], Value::Null);
    let typed = request(
        &client,
        &origin,
        &cookie,
        "POST",
        "/resources",
        json!({"kind":"bundle","data":typed_identity}),
        200,
    )
    .await;
    let typed_url = typed["data"]["subscription_url"].as_str().unwrap();
    let typed_yaml = client
        .get(typed_url)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let typed_config = camofy::engine::parse(&typed_yaml).unwrap();
    assert_eq!(typed_config["mixed-port"].as_i64(), Some(7897));
    assert_eq!(typed_config["proxies"][0]["dialer-proxy"], "Path");
    let mut invalid = typed_identity.clone();
    invalid["profiles"][1]["enabled"] = json!(false);
    request(
        &client,
        &origin,
        &cookie,
        "POST",
        "/store/identity-preview",
        json!({"data":invalid}),
        400,
    )
    .await;
    assert_eq!(
        client
            .get(typed_url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        typed_yaml
    );
    server.abort();
}
