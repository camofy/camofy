use crate::{App, Error, history, retry, security, store, usage, westdata};
use serde_json::{Value, json};
use sqlx::Row;
use tracing::Instrument;
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
/// A disabled or rotated panel link answers 401/403/404; retrying it unchanged is pointless.
fn rejected(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<reqwest::Error>()
        .and_then(reqwest::Error::status)
        .is_some_and(|code| matches!(code.as_u16(), 401 | 403 | 404))
}

#[cfg(test)]
mod tests {
    use super::rejected;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Only a rejected panel link invalidates the resolved address; transient and server
    /// errors keep it so the retry re-fetches instead of logging in again.
    #[tokio::test]
    async fn only_link_rejections_invalidate_a_resolved_panel_address() {
        for (status, expected) in [
            (401u16, true),
            (403, true),
            (404, true),
            (429, false),
            (500, false),
            (502, false),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 1024];
                let _ = socket.read(&mut buffer).await.unwrap();
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status} X\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx"
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            });
            let response = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(format!("http://{address}/sub"))
                .send()
                .await
                .unwrap();
            let error = crate::retry::check_http(&response).unwrap_err();
            assert_eq!(rejected(&error), expected, "status {status}");
            server.await.unwrap();
        }
        assert!(!rejected(&anyhow::anyhow!("transport failure")));
    }
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
    let (profile, policy) = {
        let mut conn = app.db.acquire().await?;
        let p = store::get(app, &mut conn, user, id).await?;
        let proxy = crate::admin::snapshot(app, &mut conn).await?;
        (p, proxy)
    };
    let mut stage = "proxy";
    let mut attempts = 0;
    let mut failures = Vec::new();
    // A Zyte-driven panel refresh gets a longer attempt. Cheap proxy preparation can retry before
    // any login starts, but network retries must not repeat the expensive panel conversation.
    let slow_panel = westdata::config(&profile.data).is_some() && crate::zyte::Zyte::configured();
    let max_attempts = retry::ATTEMPTS;
    let attempt_timeout = if slow_panel {
        retry::PANEL_ATTEMPT_TIMEOUT
    } else {
        retry::ATTEMPT_TIMEOUT
    };
    let total_timeout = if slow_panel {
        retry::PANEL_TOTAL_TIMEOUT
    } else {
        retry::TOTAL_TIMEOUT
    };
    // A panel login costs a captcha and its activation window lasts ten minutes, so one
    // resolution serves every retry inside this refresh.
    let mut panel: Option<(westdata::Resolved, tokio::time::Instant)> = None;
    if slow_panel {
        // The claim was taken for a plain fetch; a panel refresh needs the job for longer, and a
        // second worker must not pick it up while the captcha and browser round trips are running.
        sqlx::query("UPDATE fetch_jobs SET leased_until=now()+interval '240 seconds' WHERE profile_id=$1 AND claim=$2")
            .bind(id)
            .bind(claim)
            .execute(&app.db)
            .await?;
    }
    let deadline = tokio::time::Instant::now() + total_timeout;
    let fetch = async {
        loop {
            attempts += 1;
            stage = "proxy";
            let attempt = async {
                let endpoint = match policy.proxy.as_ref() {
                    Some(p) => {
                        crate::provider::extraction_slot(app, &p.data).await?;
                        Some(crate::provider::resolve(&p.data, app.private_egress).await?)
                    }
                    None if policy.direct => None,
                    None => return Err(anyhow::anyhow!("platform egress unavailable")),
                };
                let url = match westdata::config(&profile.data) {
                    Some(cfg) => match &panel {
                        Some((resolved, at)) if at.elapsed() < retry::PANEL_REUSE => {
                            resolved.clash.clone()
                        }
                        _ => {
                            // Log in, read the current Clash address, open the update switch.
                            stage = "westdata";
                            let vision = app.captcha.as_ref().ok_or_else(|| {
                                anyhow::anyhow!("未配置验证码识别模型，无法刷新 WestData 订阅")
                            })?;
                            let resolved: westdata::Resolved = westdata::resolve(
                                vision,
                                endpoint.as_deref(),
                                app.private_egress,
                                &cfg,
                            )
                            .await?;
                            let clash = resolved.clash.clone();
                            panel = Some((resolved, tokio::time::Instant::now()));
                            clash
                        }
                    },
                    // Managed, but the account block is incomplete: never fall back to a
                    // stale address that the panel has already rotated.
                    None if westdata::is_managed(&profile.data) => {
                        stage = "westdata";
                        return Err(anyhow::Error::new(westdata::Failure {
                            step: "westdata_login",
                            message: "WestData 账号信息不完整，请重新填写账号与密码。",
                        }));
                    }
                    None => profile.data["url"].as_str().unwrap_or("").to_string(),
                };
                stage = "fetch";
                let outcome = security::fetch(&url, endpoint.as_deref(), app.private_egress).await;
                if let Err(error) = &outcome
                    && rejected(error)
                {
                    // A disabled or rotated panel link must be re-resolved, not retried as-is.
                    panel = None;
                }
                outcome.map(|fetched| (url, fetched))
            }
            .instrument(
                tracing::info_span!("subscription_refresh_attempt", %claim, attempt = attempts),
            );
            let result = match tokio::time::timeout(attempt_timeout, attempt).await {
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
            tracing::warn!(%claim, stage, attempt = attempts, code, "subscription refresh attempt failed");
            failures.push(format!("第 {attempts} 次：{code}"));
            let Some(server_delay) = retry::delay(error) else {
                break result;
            };
            if attempts >= max_attempts || !retry::may_repeat(slow_panel, stage) {
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
                "已尝试 {attempts}/{max_attempts} 次（{code}），约 {} 秒后自动重试。",
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
            let same_proxy = policy.same(
                &crate::admin::snapshot(app, &mut conn)
                    .await
                    .map_err(|_| anyhow::anyhow!("platform egress unavailable"))?,
            );
            if !active || !unchanged || !same_proxy {
                break result;
            }
        }
    };
    let changed = async {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let mut conn = app.db.acquire().await?;
            if !policy.same(&crate::admin::snapshot(app, &mut conn).await?) {
                break;
            }
        }
        Ok::<(), Error>(())
    };
    let result = match tokio::time::timeout(total_timeout, async {
        tokio::select! {
            result = fetch => result,
            _ = changed => Err(anyhow::anyhow!("platform egress changed or unavailable")),
        }
    })
    .await
    {
        Ok(result) => result,
        Err(_) => {
            stage = "timeout";
            Err(anyhow::anyhow!("refresh deadline exceeded"))
        }
    };
    let mut tx = app.db.begin().await?;
    // Serialize result publication with policy changes, across all tenants/replicas.
    sqlx::query("SELECT pg_advisory_xact_lock_shared(739214801)")
        .execute(&mut *tx)
        .await?;
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
    let proxy_unchanged = policy.same(&crate::admin::snapshot(app, &mut tx).await?);
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
    current.data.as_object_mut().unwrap().remove("last_proxy");
    current
        .data
        .as_object_mut()
        .unwrap()
        .remove("last_proxy_at");
    let old_content = current.data["content"].clone();
    let failure = match result {
        Ok((url, fetched)) => {
            // The panel decides this address; persist what was actually fetched.
            current.data["url"] = json!(url);
            if let Some((resolved, _)) = &panel
                && let Some(entry) = current.data["westdata"].as_object_mut()
            {
                entry.insert("subscription_url".into(), json!(resolved.base.as_str()));
                entry.insert("last_activate".into(), json!(crate::now()));
            }
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
    let retry_scope = if slow_panel {
        "（仅获取代理阶段自动重试）"
    } else {
        ""
    };
    let message = if attempts > 1 || !failures.is_empty() {
        Some(format!(
            "{} 已尝试 {attempts}/{} 次{retry_scope}。{}",
            failure
                .as_ref()
                .map(|(_, m)| m.as_str())
                .unwrap_or("重试成功，已更新订阅内容。"),
            max_attempts,
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
