use crate::{App, Error};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use rand_core::OsRng;
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct Credentials {
    email: String,
    password: String,
    nickname: Option<String>,
}
pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}
fn cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|s| s.trim().strip_prefix("camofy_session="))
}
pub async fn user(app: &App, headers: &HeaderMap, write: bool) -> Result<Uuid, Error> {
    if write && bearer(headers).is_none() {
        let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
        if !origin.is_some_and(|o| app.accepts_origin(o)) {
            return Err(Error::new(StatusCode::FORBIDDEN, "invalid request origin"));
        }
    }
    let token = bearer(headers)
        .or_else(|| cookie(headers))
        .ok_or_else(Error::unauthorized)?;
    sqlx::query_scalar("SELECT user_id FROM sessions WHERE hash=$1 AND expires_at>now()")
        .bind(camofy::digest(token))
        .fetch_optional(&app.db)
        .await?
        .ok_or_else(Error::unauthorized)
}
pub async fn rate(app: &App, key: String, limit: i32, seconds: i32) -> Result<(), Error> {
    let count:i32=sqlx::query_scalar("INSERT INTO rate_limits(key,count,expires_at) VALUES($1,1,now()+make_interval(secs=>$2)) ON CONFLICT(key) DO UPDATE SET count=CASE WHEN rate_limits.expires_at<now() THEN 1 ELSE rate_limits.count+1 END, expires_at=CASE WHEN rate_limits.expires_at<now() THEN EXCLUDED.expires_at ELSE rate_limits.expires_at END RETURNING count")
        .bind(key).bind(seconds as f64).fetch_one(&app.db).await?;
    if count > limit {
        return Err(Error::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit exceeded",
        ));
    }
    Ok(())
}
pub async fn register(
    State(app): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(c): Json<Credentials>,
) -> Result<Response, Error> {
    if !app.registration {
        return Err(Error::new(StatusCode::FORBIDDEN, "registration disabled"));
    }
    rate(&app, format!("register:{}", addr.ip()), 10, 3600).await?;
    credentials(app, c, true).await
}
pub async fn login(
    State(app): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(c): Json<Credentials>,
) -> Result<Response, Error> {
    rate(&app, format!("login:{}", addr.ip()), 30, 300).await?;
    rate(
        &app,
        format!(
            "login-email:{}",
            camofy::digest(c.email.trim().to_lowercase())
        ),
        20,
        300,
    )
    .await?;
    credentials(app, c, false).await
}
async fn credentials(app: App, c: Credentials, register: bool) -> Result<Response, Error> {
    let email = c.email.trim().to_lowercase();
    if email.len() > 254 || !email.contains('@') || c.password.len() < 12 || c.password.len() > 256
    {
        return Err(Error::bad(
            "valid email and password of 12–256 bytes required",
        ));
    }
    // Bound memory-hard hashes independently of HTTP concurrency.
    let permit = app
        .hash_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| Error::bad("server shutting down"))?;
    let nickname = c
        .nickname
        .unwrap_or_else(|| "新用户".into())
        .trim()
        .to_owned();
    if register {
        validate_nickname(&nickname)?;
    }
    let id = if register {
        let hash = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            Argon2::default()
                .hash_password(c.password.as_bytes(), &SaltString::generate(&mut OsRng))
                .map(|s| s.to_string())
                .map_err(|_| Error::bad("password hashing failed"))
        })
        .await??;
        let id = Uuid::new_v4();
        let result =
            sqlx::query("INSERT INTO users(id,email,password,nickname) VALUES($1,$2,$3,$4)")
                .bind(id)
                .bind(&email)
                .bind(hash)
                .bind(&nickname)
                .execute(&app.db)
                .await;
        if let Err(sqlx::Error::Database(e)) = &result
            && e.is_unique_violation()
        {
            return Err(Error::new(StatusCode::CONFLICT, "account already exists"));
        }
        result?;
        id
    } else {
        let row: Option<(Uuid, String)> =
            sqlx::query_as("SELECT id,password FROM users WHERE email=$1")
                .bind(&email)
                .fetch_optional(&app.db)
                .await?;
        let Some((id, hash)) = row else {
            return Err(Error::unauthorized());
        };
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let hash = PasswordHash::new(&hash).map_err(|_| Error::unauthorized())?;
            Argon2::default()
                .verify_password(c.password.as_bytes(), &hash)
                .map_err(|_| Error::unauthorized())
        })
        .await??;
        id
    };
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO sessions(hash,user_id,expires_at) VALUES($1,$2,now()+interval '7 days')",
    )
    .bind(camofy::digest(&token))
    .bind(id)
    .execute(&app.db)
    .await?;
    let nickname: String = sqlx::query_scalar("SELECT nickname FROM users WHERE id=$1")
        .bind(id)
        .fetch_one(&app.db)
        .await?;
    let mut response =
        Json(json!({"user_id":id,"email":email,"nickname":nickname})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        format!(
            "camofy_session={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=604800{}",
            if app.secure { "; Secure" } else { "" }
        )
        .parse()
        .unwrap(),
    );
    Ok(response)
}
pub async fn me(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, Error> {
    let id = user(&app, &headers, false).await?;
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id=$1")
        .bind(id)
        .fetch_one(&app.db)
        .await?;
    let nickname: String = sqlx::query_scalar("SELECT nickname FROM users WHERE id=$1")
        .bind(id)
        .fetch_one(&app.db)
        .await?;
    Ok(Json(
        json!({"user_id":id,"email":email,"nickname":nickname}),
    ))
}
fn validate_nickname(s: &str) -> Result<(), Error> {
    if s.is_empty() || s.chars().count() > 40 || s.chars().any(char::is_control) {
        return Err(Error::bad("昵称需要 1–40 个字符，不能包含控制字符。"));
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountEdit {
    nickname: String,
}
pub async fn update_account(
    State(app): State<App>,
    headers: HeaderMap,
    Json(edit): Json<AccountEdit>,
) -> Result<Json<serde_json::Value>, Error> {
    let id = user(&app, &headers, true).await?;
    rate(&app, format!("account:{id}"), 20, 60).await?;
    let name = edit.nickname.trim();
    validate_nickname(name)?;
    let mut tx = app.db.begin().await?;
    let email: String =
        sqlx::query_scalar("UPDATE users SET nickname=$2 WHERE id=$1 RETURNING email")
            .bind(id)
            .bind(name)
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query("UPDATE catalog_publishers SET display_name=$2 WHERE user_id=$1")
        .bind(id)
        .bind(name)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE catalog_packages SET publisher=$2 WHERE owner_id=$1")
        .bind(id)
        .bind(name)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({"user_id":id,"email":email,"nickname":name})))
}
pub async fn logout(State(app): State<App>, headers: HeaderMap) -> Result<Response, Error> {
    user(&app, &headers, true).await?;
    if let Some(token) = bearer(&headers).or_else(|| cookie(&headers)) {
        sqlx::query("DELETE FROM sessions WHERE hash=$1")
            .bind(camofy::digest(token))
            .execute(&app.db)
            .await?;
    }
    let mut r = StatusCode::NO_CONTENT.into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        "camofy_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"
            .parse()
            .unwrap(),
    );
    Ok(r)
}
