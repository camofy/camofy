use super::*;
use axum::{
    extract::{Path, State},
    http::HeaderMap,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

#[test]
fn agent_hash_is_independent_of_selection_and_stale_defaults_do_not_block_publish() {
    let p = store::Resource {
        id: Uuid::new_v4(),
        kind: "profile".into(),
        version: 1,
        data: json!({"content":"proxy-groups: [{name: pick, type: select, proxies: [DIRECT, REJECT]}]\nrules: ['MATCH,pick']"}),
    };
    let mut b =
        json!({"profiles":[{"profile_id":p.id,"enabled":true}],"selections":{"pick":"DIRECT"}});
    let a = store::render_bundle(std::slice::from_ref(&p), &b, "https://cloud.example")
        .unwrap()
        .0;
    b["selections"]["pick"] = json!("REJECT");
    let next = store::render_bundle(std::slice::from_ref(&p), &b, "https://cloud.example")
        .unwrap()
        .0;
    assert_eq!(a["agent"]["hash"], next["agent"]["hash"]);
    assert_ne!(a["router"]["hash"], next["router"]["hash"]);
    b["selections"]["pick"] = json!("removed-by-upstream");
    assert!(store::render_bundle(&[p], &b, "https://cloud.example").is_ok());
}

#[tokio::test]
#[ignore = "requires disposable TEST_DATABASE_URL"]
async fn control_end_to_end() {
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&db).await.unwrap();
    let app = App {
        db: db.clone(),
        vault: security::Vault::new(&STANDARD.encode([9; 32])).unwrap(),
        origin: "http://localhost:3990".into(),
        legacy_origins: vec![],
        secure: false,
        registration: true,
        private_egress: true,
        workers: 1,
        topics: Default::default(),
        hash_slots: Arc::new(Semaphore::new(1)),
    };
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,email,password) VALUES($1,$2,'unused')")
        .bind(user)
        .bind(format!("{user}@example.test"))
        .execute(&db)
        .await
        .unwrap();
    let session = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO sessions(hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 hour')",
    )
    .bind(camofy::digest(&session))
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
    let mut h = HeaderMap::new();
    h.insert(
        "authorization",
        format!("Bearer {session}").parse().unwrap(),
    );
    let p = store::Resource {
        id: Uuid::new_v4(),
        kind: "profile".into(),
        version: 1,
        data: json!({"type":"overlay","name":"fixture","content":"proxy-groups: [{name: pick, type: select, proxies: [DIRECT, REJECT]}]\nrules: ['MATCH,pick']"}),
    };
    let b = store::Resource {
        id: Uuid::new_v4(),
        kind: "bundle".into(),
        version: 1,
        data: json!({"name":"test identity","profiles":[{"profile_id":p.id,"enabled":true}],"selections":{"pick":"DIRECT"}}),
    };
    let mut d = store::Resource {
        id: Uuid::new_v4(),
        kind: "device".into(),
        version: 1,
        data: json!({"name":"test device","bundle_id":b.id,"reported":{"protocol":2}}),
    };
    let mut conn = db.acquire().await.unwrap();
    for r in [&p, &b, &d] {
        store::put(&app, &mut conn, user, r).await.unwrap();
    }
    store::rebuild(&app, &mut conn, user).await.unwrap();
    drop(conn);
    let edit = || control::SelectionEdit {
        expected_version: 0,
        selections: [("pick".into(), "REJECT".into())].into(),
    };
    assert!(
        control::select(State(app.clone()), h.clone(), Path(b.id), Json(edit()))
            .await
            .is_ok()
    );
    assert_eq!(
        control::select(State(app.clone()), h.clone(), Path(b.id), Json(edit()))
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    let mut conn = db.acquire().await.unwrap();
    let changed = store::get(&app, &mut conn, user, b.id).await.unwrap();
    assert_eq!(changed.data["selection_version"], 1);
    drop(conn);
    let request = || camofy::protocol::RpcRequest {
        method: "core.restart".into(),
        params: Value::Null,
        idempotency_key: "restart-once".into(),
    };
    let job = control::enqueue(State(app.clone()), h.clone(), Path(d.id), Json(request()))
        .await
        .unwrap()
        .0;
    let duplicate = control::enqueue(State(app.clone()), h.clone(), Path(d.id), Json(request()))
        .await
        .unwrap()
        .0;
    assert_eq!(job.id, duplicate.id);
    assert_eq!(job.expires_at - job.created_at, 600);
    let token = "c".repeat(64);
    sqlx::query("INSERT INTO access_tokens(hash,user_id,bundle_id,device_id,label) VALUES($1,$2,$3,$4,'control test')").bind(camofy::digest(&token)).bind(user).bind(b.id).bind(d.id).execute(&db).await.unwrap();
    let mut agent = HeaderMap::new();
    agent.insert("authorization", format!("Bearer {token}").parse().unwrap());
    let report = |binding: String, status: &str| control::AgentReport {
        binding,
        state: None,
        job_id: Some(job.id.clone()),
        status: Some(status.into()),
        result: Some(json!({"safe":true})),
    };
    assert_eq!(
        control::agent_report(
            State(app.clone()),
            agent.clone(),
            Json(report("old-binding".into(), "executing"))
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::CONFLICT
    );
    assert!(
        control::agent_report(
            State(app.clone()),
            agent.clone(),
            Json(report(b.id.to_string(), "executing"))
        )
        .await
        .is_ok()
    );
    assert!(
        control::agent_report(
            State(app.clone()),
            agent.clone(),
            Json(report(b.id.to_string(), "succeeded"))
        )
        .await
        .is_ok()
    );
    assert_eq!(
        control::agent_report(
            State(app.clone()),
            agent.clone(),
            Json(report(b.id.to_string(), "executing"))
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::CONFLICT
    );
    // Identity tokens cannot report device state or change device overrides.
    sqlx::query("UPDATE access_tokens SET device_id=NULL WHERE hash=$1")
        .bind(camofy::digest(&token))
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        control::agent_report(
            State(app.clone()),
            agent.clone(),
            Json(report(b.id.to_string(), "succeeded"))
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE access_tokens SET device_id=$2 WHERE hash=$1")
        .bind(camofy::digest(&token))
        .bind(d.id)
        .execute(&db)
        .await
        .unwrap();
    // Device local override CAS and reset are independent from identity choice.
    let override_edit = |version, selections| control::OverrideEdit {
        binding: b.id.to_string(),
        expected_version: version,
        selections,
    };
    assert!(
        control::agent_overrides(
            State(app.clone()),
            agent.clone(),
            Json(override_edit(0, [("pick".into(), "DIRECT".into())].into()))
        )
        .await
        .is_ok()
    );
    assert_eq!(
        control::agent_overrides(
            State(app.clone()),
            agent.clone(),
            Json(override_edit(0, Default::default()))
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::CONFLICT
    );
    assert!(
        control::agent_overrides(
            State(app.clone()),
            agent.clone(),
            Json(override_edit(1, Default::default()))
        )
        .await
        .is_ok()
    );
    // Revoked/rebound generation rejects old receipts, even with the same identity.
    let mut conn = db.acquire().await.unwrap();
    d = store::get(&app, &mut conn, user, d.id).await.unwrap();
    d.data["binding_generation"] = json!(Uuid::new_v4());
    store::put(&app, &mut conn, user, &d).await.unwrap();
    drop(conn);
    assert_eq!(
        control::agent_report(
            State(app.clone()),
            agent.clone(),
            Json(report(b.id.to_string(), "succeeded"))
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::CONFLICT
    );
    let other = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,email,password) VALUES($1,$2,'unused')")
        .bind(other)
        .bind(format!("{other}@example.test"))
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE sessions SET user_id=$2 WHERE hash=$1")
        .bind(camofy::digest(&session))
        .bind(other)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        control::view(State(app.clone()), h.clone(), Path(d.id))
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    sqlx::query("DELETE FROM users WHERE id IN ($1,$2)")
        .bind(user)
        .bind(other)
        .execute(&db)
        .await
        .unwrap();
}
