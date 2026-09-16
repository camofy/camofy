use crate::{App, Error, security, store};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

pub async fn start(app: App) {
    // Fixed-size consumers, indexed due queue and SKIP LOCKED allow additional replicas.
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
        }
    });
}
pub async fn once(app: &App) -> Result<bool, Error> {
    let claim = Uuid::new_v4();
    let job=sqlx::query("WITH due AS (SELECT profile_id FROM fetch_jobs WHERE next_run<=now() AND leased_until<now() ORDER BY next_run FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE fetch_jobs j SET leased_until=now()+interval '120 seconds',claim=$1 FROM due WHERE j.profile_id=due.profile_id RETURNING j.profile_id,j.user_id")
        .bind(claim).fetch_optional(&app.db).await?;
    let Some(job) = job else {
        return Ok(false);
    };
    let id: Uuid = job.get("profile_id");
    let user: Uuid = job.get("user_id");
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
    let fetch = async {
        let endpoint = if let Some(p) = &proxy {
            // No endpoint cache and no direct fallback: every job extracts its own fresh IP.
            crate::provider::extraction_slot(app, &p.data).await?;
            let endpoint = crate::provider::resolve(&p.data, app.private_egress).await?;
            if p.data["provider"] == "xiequ" {
                last_proxy = Some((endpoint.clone(), crate::now()));
            }
            Some(endpoint)
        } else {
            None
        };
        security::fetch(
            profile.data["url"].as_str().unwrap_or(""),
            endpoint.as_deref(),
            app.private_egress,
        )
        .await
    };
    // Includes supplier DNS/queue/extraction; always finish before the 120-second job lease.
    let result = tokio::time::timeout(std::time::Duration::from_secs(100), fetch)
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("subscription refresh timed out")));
    let mut tx = app.db.begin().await?;
    store::lock(&mut tx, user).await?;
    let owns: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM fetch_jobs WHERE profile_id=$1 AND claim=$2)",
    )
    .bind(id)
    .bind(claim)
    .fetch_one(&mut *tx)
    .await?;
    if !owns {
        return Ok(true);
    }
    let current = store::get(app, &mut tx, user, id).await;
    let Ok(mut current) = current else {
        return Ok(true);
    };
    let proxy_unchanged = if let Some(p) = &proxy {
        store::get(app, &mut tx, user, p.id)
            .await
            .is_ok_and(|x| x.version == p.version)
    } else {
        true
    };
    if current.version != profile.version || !proxy_unchanged {
        sqlx::query("UPDATE fetch_jobs SET leased_until='-infinity',next_run=now() WHERE profile_id=$1 AND claim=$2").bind(id).bind(claim).execute(&mut *tx).await?;
        tx.commit().await?;
        return Ok(true);
    }
    current.data["last_fetch"] = json!(crate::now());
    current.data["last_proxy"] = json!(last_proxy.as_ref().map(|p| &p.0));
    current.data["last_proxy_at"] = json!(last_proxy.map(|p| p.1));
    match result {
        Ok(content) => {
            current.data["content"] = json!(content);
            current.data["fetch_status"] = json!("ok");
            current.data["error"] = serde_json::Value::Null;
        }
        Err(e) => {
            // Do not store reqwest's Display: it contains upstream URLs and credentials.
            let message = if let Some(request) = e.downcast_ref::<reqwest::Error>() {
                if let Some(status) = request.status() {
                    format!(
                        "subscription returned HTTP {}; check upstream access/expiry",
                        status.as_u16()
                    )
                } else if request.is_timeout() {
                    "subscription/proxy request timed out".to_string()
                } else {
                    let io = e
                        .chain()
                        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
                        .map(|cause| format!(" ({:?})", cause.kind()))
                        .unwrap_or_default();
                    format!(
                        "subscription/proxy transport failed{io}; check proxy reachability and upstream TLS"
                    )
                }
            } else {
                e.to_string()
            };
            current.data["fetch_status"] = json!("error");
            current.data["error"] = json!(message);
        }
    }
    store::put(app, &mut tx, user, &current).await?;
    store::rebuild(app, &mut tx, user).await?;
    if current.data["auto_refresh"].as_bool().unwrap_or(true) {
        let interval = current.data["interval_seconds"].as_i64().unwrap_or(3600) as f64;
        sqlx::query("UPDATE fetch_jobs SET next_run=now()+make_interval(secs=>$3)+(random()*interval '20 seconds'), leased_until='-infinity',claim=NULL WHERE profile_id=$1 AND claim=$2").bind(id).bind(claim).bind(interval).execute(&mut *tx).await?;
    } else {
        sqlx::query("DELETE FROM fetch_jobs WHERE profile_id=$1 AND claim=$2")
            .bind(id)
            .bind(claim)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(true)
}
