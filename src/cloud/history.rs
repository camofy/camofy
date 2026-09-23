use crate::{App, Error, auth, store};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct Page {
    before: Option<i64>,
}
pub async fn list(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(page): Query<Page>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, false).await?;
    let mut conn = app.db.acquire().await?;
    let p = store::get(&app, &mut conn, user, id).await?;
    if p.kind != "profile" || p.data["type"] != "source" {
        return Err(Error::not_found());
    }
    let rows = sqlx::query("SELECT id,extract(epoch from started_at)::bigint AS started,extract(epoch from finished_at)::bigint AS finished,duration_ms,reason,status,error_code,message,usage_status FROM refresh_history WHERE user_id=$1 AND profile_id=$2 AND ($3::bigint IS NULL OR id<$3) AND started_at>now()-interval '30 days' ORDER BY id DESC LIMIT 21")
        .bind(user).bind(id).bind(page.before).fetch_all(&mut *conn).await?;
    let items: Vec<_> = rows.iter().take(20).map(|r| json!({
        "id":r.get::<i64,_>("id").to_string(), "started_at":r.get::<i64,_>("started"), "finished_at":r.get::<Option<i64>,_>("finished"),
        "duration_ms":r.get::<Option<i64>,_>("duration_ms"), "reason":r.get::<String,_>("reason"), "status":r.get::<String,_>("status"),
        "error_code":r.get::<Option<String>,_>("error_code"), "message":r.get::<Option<String>,_>("message"), "usage_status":r.get::<Option<String>,_>("usage_status")
    })).collect();
    let next = if rows.len() > 20 {
        items.last().map(|v| v["id"].clone())
    } else {
        None
    };
    Ok(Json(
        json!({"items":items,"next_cursor":next,"retention_days":30,"retention_count":100}),
    ))
}
pub async fn finish(
    conn: &mut sqlx::PgConnection,
    claim: Uuid,
    status: &str,
    code: Option<&str>,
    message: Option<&str>,
    usage: Option<&str>,
) -> Result<(), Error> {
    sqlx::query("UPDATE refresh_history SET finished_at=clock_timestamp(),duration_ms=greatest(0,(extract(epoch from clock_timestamp()-started_at)*1000)::bigint),status=$2,error_code=$3,message=$4,usage_status=$5 WHERE claim=$1 AND status='running'")
        .bind(claim).bind(status).bind(code).bind(message).bind(usage).execute(conn).await?;
    Ok(())
}
pub async fn prune(conn: &mut sqlx::PgConnection, user: Uuid, profile: Uuid) -> Result<(), Error> {
    sqlx::query("DELETE FROM refresh_history WHERE user_id=$1 AND profile_id=$2 AND status<>'running' AND (started_at<now()-interval '30 days' OR id IN (SELECT id FROM refresh_history WHERE user_id=$1 AND profile_id=$2 ORDER BY id DESC OFFSET 100))")
        .bind(user).bind(profile).execute(conn).await?;
    Ok(())
}

/// Never store upstream bodies, URLs, parser snippets or provider error Display.
pub fn failure(error: &anyhow::Error, stage: &str) -> (&'static str, String) {
    // Panel conversations carry their own fixed, secret-free wording and a precise step.
    if let Some(panel) = error.downcast_ref::<crate::westdata::Failure>() {
        return (panel.step, panel.message.to_string());
    }
    if let Some(e) = error.downcast_ref::<reqwest::Error>() {
        if let Some(status) = e.status() {
            return (
                "upstream_http",
                format!(
                    "上游返回 HTTP {}，请检查订阅权限、有效期或服务状态。",
                    status.as_u16()
                ),
            );
        }
        if e.is_timeout() {
            return ("timeout", "订阅或代理请求超时，已保留上次成功配置。".into());
        }
    }
    match stage {
        "attempt_timeout" => (
            "timeout",
            "单次刷新超过 30 秒时限，已保留上次成功配置。".into(),
        ),
        "proxy" => (
            "proxy_failure",
            "平台订阅出口暂不可用，已保留上次成功配置，请稍后重试。".into(),
        ),
        "westdata" => (
            "westdata_failure",
            "WestData 面板登录或订阅地址读取失败，已保留上次成功配置。".into(),
        ),
        "timeout" => (
            "timeout",
            "刷新超过 100 秒时限，已保留上次成功配置。".into(),
        ),
        _ => (
            "fetch_failure",
            "拉取失败，请检查订阅/代理网络、TLS、目标地址或响应是否超过 4 MiB。".into(),
        ),
    }
}
