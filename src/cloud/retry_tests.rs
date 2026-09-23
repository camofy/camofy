use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Isolated DB and a local fake proxy: never spends supplier credits or requests a real airport.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to disposable PostgreSQL"]
async fn subscription_retry_end_to_end() {
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&db).await.unwrap();
    let app = App {
        db: db.clone(),
        vault: security::Vault::new(&STANDARD.encode([7; 32])).unwrap(),
        origin: "https://cloud.example".into(),
        legacy_origins: vec![],
        secure: false,
        registration: true,
        private_egress: true,
        workers: 1,
        captcha: None,
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
    let hits = Arc::new(AtomicUsize::new(0));
    let fail_until = Arc::new(AtomicUsize::new(0));
    let status = Arc::new(AtomicUsize::new(503));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(
        axum::serve(
            listener,
            Router::new().fallback(get({
                let hits = hits.clone();
                let fail_until = fail_until.clone();
                let status = status.clone();
                move || {
                    let hits = hits.clone();
                    let fail_until = fail_until.clone();
                    let status = status.clone();
                    async move {
                        let n = hits.fetch_add(1, Ordering::SeqCst) + 1;
                        let code = if n <= fail_until.load(Ordering::SeqCst) {
                            status.load(Ordering::SeqCst) as u16
                        } else {
                            200
                        };
                        (
                            axum::http::StatusCode::from_u16(code).unwrap(),
                            "rules: ['MATCH,DIRECT']\n",
                        )
                    }
                }
            })),
        )
        .into_future(),
    );
    let proxy = store::Resource {
        id: Uuid::new_v4(),
        kind: "proxy".into(),
        version: 1,
        data: json!({"provider":"static","url":format!("http://{addr}")}),
    };
    let source = store::Resource {
        id: Uuid::new_v4(),
        kind: "profile".into(),
        version: 1,
        data: json!({"name":"retry-test","type":"source","url":"http://localhost:59999/private-subscription","auto_refresh":false,"content":"rules: ['MATCH,REJECT']\n"}),
    };
    let mut conn = db.acquire().await.unwrap();
    sqlx::query("INSERT INTO platform_proxies(id,data) VALUES($1,$2)")
        .bind(proxy.id)
        .bind(app.vault.seal(&proxy.data).unwrap())
        .execute(&mut *conn)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE subscription_egress_policy SET proxy_id=$1,version=version+1 WHERE singleton",
    )
    .bind(proxy.id)
    .execute(&mut *conn)
    .await
    .unwrap();
    store::put(&app, &mut conn, user, &source).await.unwrap();
    drop(conn);
    async fn queue(db: &sqlx::PgPool, user: Uuid, id: Uuid) {
        sqlx::query("INSERT INTO fetch_jobs(user_id,profile_id,reason) VALUES($1,$2,'manual') ON CONFLICT(profile_id) DO UPDATE SET next_run=now(),request_id=gen_random_uuid()")
            .bind(user).bind(id).execute(db).await.unwrap();
    }
    // First request fails; the second succeeds, one logical history row.
    fail_until.store(1, Ordering::SeqCst);
    queue(&db, user, source.id).await;
    let running = tokio::spawn({
        let app = app.clone();
        async move { worker::once(&app).await.unwrap() }
    });
    let mut saw_progress = false;
    for _ in 0..100 {
        let progress: Option<String> = sqlx::query_scalar(
            "SELECT message FROM refresh_history WHERE profile_id=$1 AND status='running'",
        )
        .bind(source.id)
        .fetch_optional(&db)
        .await
        .unwrap()
        .flatten();
        if progress.is_some_and(|s| s.contains("自动重试")) {
            saw_progress = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(saw_progress, "retry progress is persisted for history UI");
    assert!(
        !worker::once(&app).await.unwrap(),
        "lease excludes competing workers during backoff"
    );
    assert!(running.await.unwrap());
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    let row: (String, String) = sqlx::query_as(
        "SELECT status,message FROM refresh_history WHERE profile_id=$1 ORDER BY id DESC LIMIT 1",
    )
    .bind(source.id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(row.0, "ok");
    assert!(row.1.contains("2/3"));
    assert!(row.1.contains("upstream_http"));
    let mut conn = db.acquire().await.unwrap();
    let cached = store::get(&app, &mut conn, user, source.id)
        .await
        .unwrap()
        .data["content"]
        .clone();
    drop(conn);
    // Transient failures exhaust exactly three attempts, retaining successful content.
    hits.store(0, Ordering::SeqCst);
    fail_until.store(usize::MAX, Ordering::SeqCst);
    queue(&db, user, source.id).await;
    worker::once(&app).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    let mut conn = db.acquire().await.unwrap();
    let failed = store::get(&app, &mut conn, user, source.id).await.unwrap();
    assert_eq!(failed.data["content"], cached);
    assert_eq!(failed.data["fetch_status"], "error");
    drop(conn);
    assert!(
        !worker::once(&app).await.unwrap(),
        "manual-only job stops after exhaustion"
    );
    // Permanent HTTP errors don't consume two more proxy attempts.
    hits.store(0, Ordering::SeqCst);
    status.store(403, Ordering::SeqCst);
    queue(&db, user, source.id).await;
    worker::once(&app).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    // A manual refresh during backoff interrupts the retry and remains queued.
    hits.store(0, Ordering::SeqCst);
    status.store(503, Ordering::SeqCst);
    queue(&db, user, source.id).await;
    let running = tokio::spawn({
        let app = app.clone();
        async move { worker::once(&app).await.unwrap() }
    });
    for _ in 0..200 {
        if hits.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    queue(&db, user, source.id).await;
    assert!(running.await.unwrap());
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    fail_until.store(0, Ordering::SeqCst);
    assert!(worker::once(&app).await.unwrap());
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    // Scheduled exhaustion reverts to the regular interval, not an endless retry loop.
    let mut conn = db.acquire().await.unwrap();
    let mut scheduled = store::get(&app, &mut conn, user, source.id).await.unwrap();
    scheduled.data["auto_refresh"] = json!(true);
    scheduled.data["interval_seconds"] = json!(300);
    store::put(&app, &mut conn, user, &scheduled).await.unwrap();
    drop(conn);
    hits.store(0, Ordering::SeqCst);
    fail_until.store(usize::MAX, Ordering::SeqCst);
    queue(&db, user, source.id).await;
    sqlx::query("UPDATE fetch_jobs SET reason='scheduled' WHERE profile_id=$1")
        .bind(source.id)
        .execute(&db)
        .await
        .unwrap();
    worker::once(&app).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    let scheduled:bool=sqlx::query_scalar("SELECT reason='scheduled' AND next_run>now()+interval '290 seconds' AND claim IS NULL FROM fetch_jobs WHERE profile_id=$1").bind(source.id).fetch_one(&db).await.unwrap();
    assert!(scheduled);
    // A policy change during backoff cancels retries; it never falls back to direct.
    hits.store(0, Ordering::SeqCst);
    queue(&db, user, source.id).await;
    let running = tokio::spawn({
        let app = app.clone();
        async move { worker::once(&app).await.unwrap() }
    });
    for _ in 0..200 {
        if hits.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    sqlx::query(
        "UPDATE subscription_egress_policy SET proxy_id=NULL,version=version+1 WHERE singleton",
    )
    .execute(&db)
    .await
    .unwrap();
    assert!(running.await.unwrap());
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let discarded: String = sqlx::query_scalar(
        "SELECT status FROM refresh_history WHERE profile_id=$1 ORDER BY id DESC LIMIT 1",
    )
    .bind(source.id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(discarded, "discarded");
    // Requeued with no platform proxy: fail once without issuing any network call.
    assert!(worker::once(&app).await.unwrap());
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let messages: Vec<Option<String>> =
        sqlx::query_scalar("SELECT message FROM refresh_history WHERE profile_id=$1")
            .bind(source.id)
            .fetch_all(&db)
            .await
            .unwrap();
    for m in messages.into_iter().flatten() {
        assert!(!m.contains("private-subscription"));
        assert!(!m.contains("http://"));
    }
    sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(user)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM platform_proxies WHERE id=$1")
        .bind(proxy.id)
        .execute(&db)
        .await
        .unwrap();
    server.abort();
}
