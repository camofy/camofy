use crate::{App, Error, history, retry, security, store, usage};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

pub async fn start(app: App) {
    for _ in 0..app.workers {
        let app = app.clone();
        tokio::spawn(async move {
            loop {
                match once(&app).await {
                    Ok(true) => {}
                    Ok(false) => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
                    Err(e) => {
                        tracing::error!("fetch worker: {}", e.internal());
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    }
                }
            }
        });
    }
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
            let _ = sqlx::query("DELETE FROM sessions WHERE expires_at<now()")
                .execute(&app.db)
                .await;
            let _ = sqlx::query("DELETE FROM rate_limits WHERE expires_at<now()")
                .execute(&app.db)
                .await;
            let _ = sqlx::query("DELETE FROM refresh_history WHERE id IN (SELECT id FROM refresh_history WHERE started_at<now()-interval '30 days' ORDER BY started_at LIMIT 1000)").execute(&app.db).await;
        }
    });
}
pub async fn once(app: &App) -> Result<bool, Error> {
    let claim = Uuid::new_v4();
    let mut tx = app.db.begin().await?;
    let job = sqlx::query("WITH due AS (SELECT profile_id FROM fetch_jobs WHERE next_run<=now() AND leased_until<now() ORDER BY next_run FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE fetch_jobs j SET leased_until=now()+interval '120 seconds',claim=$1 FROM due WHERE j.profile_id=due.profile_id RETURNING j.profile_id,j.user_id,j.reason,j.request_id")
        .bind(claim).fetch_optional(&mut *tx).await?;
    let Some(job) = job else {
        return Ok(false);
    };
    let id: Uuid = job.get("profile_id");
    let user: Uuid = job.get("user_id");
    let request_id: Uuid = job.get("request_id");
    sqlx::query("UPDATE refresh_history SET status='error',error_code='worker_interrupted',message='上一次刷新进程中断或租约超时，正在重新尝试。',finished_at=clock_timestamp(),duration_ms=greatest(0,(extract(epoch from clock_timestamp()-started_at)*1000)::bigint) WHERE profile_id=$1 AND user_id=$2 AND status='running'")
        .bind(id).bind(user).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO refresh_history(user_id,profile_id,claim,reason) VALUES($1,$2,$3,$4)")
        .bind(user)
        .bind(id)
        .bind(claim)
        .bind(job.get::<String, _>("reason"))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let (profile, proxy) = {
        let mut conn = app.db.acquire().await?;
        let p = store::get(app, &mut conn, user, id).await?;
        let proxy = if let Some(s) = p.data["proxy_id"].as_str() {
            Some(
                store::get(
                    app,
                    &mut conn,
                    user,
                    Uuid::parse_str(s).map_err(|_| Error::bad("invalid proxy ID"))?,
                )
                .await?,
            )
        } else {
            None
        };
        (p, proxy)
    };
    let mut last_proxy = None;
    let mut stage = "proxy";
    let mut attempts = 0;
    let mut failures = Vec::new();
    let deadline = tokio::time::Instant::now() + retry::TOTAL_TIMEOUT;
    let fetch = async {
        loop {
            attempts += 1;
            stage = "proxy";
            let attempt = async {
                let endpoint = if let Some(p) = &proxy {
                    crate::provider::extraction_slot(app, &p.data).await?;
                    let endpoint = crate::provider::resolve(&p.data, app.private_egress).await?;
                    if p.data["provider"] == "xiequ" {
                        last_proxy = Some((endpoint.clone(), crate::now()));
                    }
                    Some(endpoint)
                } else {
                    None
                };
                stage = "fetch";
                security::fetch(
                    profile.data["url"].as_str().unwrap_or(""),
                    endpoint.as_deref(),
                    app.private_egress,
                )
                .await
            };
            let result = match tokio::time::timeout(retry::ATTEMPT_TIMEOUT, attempt).await {
                Ok(result) => result,
                Err(e) => {
                    stage = "attempt_timeout";
                    Err(anyhow::Error::new(e))
                }
            };
            let Err(ref error) = result else {
                break result;
            };
            let (code, _) = history::failure(error, stage);
            failures.push(format!("第 {attempts} 次：{code}"));
            let Some(server_delay) = retry::delay(error) else {
                break result;
            };
            if attempts >= retry::ATTEMPTS {
                break result;
            }
            let jitter = (Uuid::new_v4().as_u128() % 1000) as u64;
            let wait = retry::backoff(attempts, jitter, server_delay);
            // Don't violate Retry-After or hold a lease for a retry that cannot start.
            if wait >= deadline.saturating_duration_since(tokio::time::Instant::now()) {
                break result;
            }
            sqlx::query(
                "UPDATE refresh_history SET message=$2 WHERE claim=$1 AND status='running'",
            )
            .bind(claim)
            .bind(format!(
                "已尝试 {attempts}/{} 次（{code}），约 {} 秒后自动重试。",
                retry::ATTEMPTS,
                wait.as_secs() + 1
            ))
            .execute(&app.db)
            .await?;
            tokio::time::sleep(wait).await;
            // Superseded requests and configuration changes must not spend another proxy IP.
            let mut conn = app.db.acquire().await?;
            let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM fetch_jobs WHERE profile_id=$1 AND claim=$2 AND request_id=$3 AND leased_until>now())")
            .bind(id).bind(claim).bind(request_id).fetch_one(&mut *conn).await?;
            let unchanged = store::get(app, &mut conn, user, id)
                .await
                .is_ok_and(|p| p.version == profile.version);
            let same_proxy = if let Some(p) = &proxy {
                store::get(app, &mut conn, user, p.id)
                    .await
                    .is_ok_and(|r| r.version == p.version)
            } else {
                true
            };
            if !active || !unchanged || !same_proxy {
                break result;
            }
        }
    };
    let result = match tokio::time::timeout(retry::TOTAL_TIMEOUT, fetch).await {
        Ok(result) => result,
        Err(_) => {
            stage = "timeout";
            Err(anyhow::anyhow!("refresh deadline exceeded"))
        }
    };
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let pending: Option<bool> = sqlx::query_scalar(
        "SELECT request_id<>$3 FROM fetch_jobs WHERE profile_id=$1 AND claim=$2 FOR UPDATE",
    )
    .bind(id)
    .bind(claim)
    .bind(request_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(pending) = pending else {
        history::finish(
            &mut tx,
            claim,
            "discarded",
            Some("lease_lost"),
            Some("任务已被替换，未应用旧结果。"),
            None,
        )
        .await?;
        tx.commit().await?;
        return Ok(true);
    };
    let mut current = store::get(app, &mut tx, user, id).await?;
    let proxy_unchanged = if let Some(p) = &proxy {
        store::get(app, &mut tx, user, p.id)
            .await
            .is_ok_and(|x| x.version == p.version)
    } else {
        true
    };
    if current.version != profile.version || !proxy_unchanged {
        history::finish(
            &mut tx,
            claim,
            "discarded",
            Some("settings_changed"),
            Some("刷新过程中配置发生变化，旧结果已丢弃并重新排队。"),
            None,
        )
        .await?;
        sqlx::query("UPDATE fetch_jobs SET leased_until='-infinity',claim=NULL,next_run=now(),reason=CASE WHEN request_id=$3 THEN 'settings' ELSE reason END WHERE profile_id=$1 AND claim=$2")
            .bind(id).bind(claim).bind(request_id).execute(&mut *tx).await?;
        history::prune(&mut tx, user, id).await?;
        tx.commit().await?;
        return Ok(true);
    }
    current.data["last_fetch"] = json!(crate::now());
    current.data["last_proxy"] = json!(last_proxy.as_ref().map(|p| &p.0));
    current.data["last_proxy_at"] = json!(last_proxy.map(|p| p.1));
    let old_content = current.data["content"].clone();
    let failure = match result {
        Ok(fetched) => {
            let fingerprint = usage::fingerprint(app, user, &current);
            usage::store_snapshot(&mut current.data, fetched.usage, fingerprint.clone());
            if camofy::engine::parse(&fetched.content).is_ok() {
                current.data["content"] = json!(fetched.content);
                current.data["content_fingerprint"] = json!(fingerprint);
                None
            } else {
                Some((
                    "invalid_yaml",
                    "上游响应不是有效的 Clash YAML 映射（或超出结构限制），已保留上次成功配置。"
                        .into(),
                ))
            }
        }
        Err(e) => {
            if ["ok", "stale"].contains(&current.data["usage"]["status"].as_str().unwrap_or("")) {
                current.data["usage"]["status"] = json!("stale");
            }
            Some(history::failure(&e, stage))
        }
    };
    current.data["fetch_status"] = json!(if failure.is_some() { "error" } else { "ok" });
    current.data["error"] = failure
        .as_ref()
        .map(|(_, message)| json!(message))
        .unwrap_or(Value::Null);
    let message = if attempts > 1 || !failures.is_empty() {
        Some(format!(
            "{} 已尝试 {attempts}/{} 次。{}",
            failure
                .as_ref()
                .map(|(_, m)| m.as_str())
                .unwrap_or("重试成功，已更新订阅内容。"),
            retry::ATTEMPTS,
            failures.join("；")
        ))
    } else {
        failure.as_ref().map(|(_, m)| m.clone())
    };
    history::finish(
        &mut tx,
        claim,
        current.data["fetch_status"].as_str().unwrap(),
        failure.as_ref().map(|(code, _)| *code),
        message.as_deref(),
        current.data["usage"]["status"].as_str(),
    )
    .await?;
    store::put(app, &mut tx, user, &current).await?;
    // Metadata-only refreshes do not broadcast to agents. Backfill current legacy
    // manifests once without changing an otherwise identical configuration revision.
    let legacy: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM (SELECT DISTINCT ON(bundle_id) usage_sources FROM revisions WHERE user_id=$1 ORDER BY bundle_id,created_at DESC) latest WHERE usage_sources IS NULL)").bind(user).fetch_one(&mut *tx).await?;
    if old_content != current.data["content"]
        || legacy
        || profile.data["content_fingerprint"] != current.data["content_fingerprint"]
    {
        store::rebuild(app, &mut tx, user).await?;
    }
    if pending {
        sqlx::query("UPDATE fetch_jobs SET leased_until='-infinity',claim=NULL WHERE profile_id=$1 AND claim=$2").bind(id).bind(claim).execute(&mut *tx).await?;
    } else if current.data["auto_refresh"].as_bool().unwrap_or(true) {
        let interval = current.data["interval_seconds"].as_i64().unwrap_or(3600) as f64;
        sqlx::query("UPDATE fetch_jobs SET next_run=now()+make_interval(secs=>$3)+(random()*interval '20 seconds'),leased_until='-infinity',claim=NULL,reason='scheduled' WHERE profile_id=$1 AND claim=$2")
            .bind(id).bind(claim).bind(interval).execute(&mut *tx).await?;
    } else {
        sqlx::query("DELETE FROM fetch_jobs WHERE profile_id=$1 AND claim=$2")
            .bind(id)
            .bind(claim)
            .execute(&mut *tx)
            .await?;
    }
    history::prune(&mut tx, user, id).await?;
    tx.commit().await?;
    Ok(true)
}
