//! OAuth 2.0 device authorization for first-time Agent provisioning (RFC 8628).
//! Browser carries only a short-lived user code; device_code never leaves the Agent.
use crate::{App, Error, auth, store};
use axum::{
    Form, Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use std::net::{IpAddr, SocketAddr};
use uuid::Uuid;

const CLIENT: &str = "camofy-agent";
const SCOPE: &str = "device:sync";
fn secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
fn opaque(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit())
}
fn code_hash(s: &str) -> Result<String, Error> {
    let s = s.replace('-', "").to_ascii_lowercase();
    if s.len() != 16 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::bad("invalid_user_code"));
    }
    Ok(camofy::digest(s))
}
fn local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_private() || v.is_loopback(),
        IpAddr::V6(v) => v.is_loopback() || (v.segments()[0] & 0xfe00) == 0xfc00,
    }
}
/// A return navigation is a constrained local UX extension, not an OAuth redirect.
/// Never accept an arbitrary public URL or put credentials into this navigation.
fn return_uri(value: &str) -> Result<url::Url, Error> {
    let u = url::Url::parse(value).map_err(|_| Error::bad("invalid return URI"))?;
    let local = match u.host() {
        Some(url::Host::Ipv4(ip)) => local_ip(ip.into()),
        Some(url::Host::Ipv6(ip)) => local_ip(ip.into()),
        Some(url::Host::Domain("localhost")) => true,
        _ => false,
    };
    if !local
        || !["http", "https"].contains(&u.scheme())
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || u.path() != "/bind/complete"
    {
        return Err(Error::bad(
            "return URI must be a local IP /bind/complete address",
        ));
    }
    Ok(u)
}
#[derive(Deserialize)]
pub struct DeviceRequest {
    client_id: String,
    scope: String,
    device_name: String,
    return_uri: String,
    state: String,
}
pub async fn device_authorization(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Form(r): Form<DeviceRequest>,
) -> Result<Json<Value>, Error> {
    auth::rate(&app, format!("device-start:{}", peer.ip()), 60, 300).await?;
    if r.client_id != CLIENT
        || r.scope != SCOPE
        || !opaque(&r.state)
        || r.device_name.trim().is_empty()
        || r.device_name.len() > 120
    {
        return Err(Error::bad("invalid_request"));
    }
    let callback = return_uri(&r.return_uri)?;
    let device_code = secret();
    let raw = Uuid::new_v4().simple().to_string()[..16].to_uppercase();
    let user_code = format!("{}-{}", &raw[..8], &raw[8..]);
    sqlx::query("DELETE FROM device_authorizations WHERE expires_at < now()")
        .execute(&app.db)
        .await?;
    sqlx::query("INSERT INTO device_authorizations(device_hash,user_code_hash,device_name,return_uri,state,expires_at) VALUES($1,$2,$3,$4,$5,now()+interval '10 minutes')")
        .bind(camofy::digest(&device_code)).bind(code_hash(&user_code)?).bind(r.device_name.trim()).bind(callback.as_str()).bind(r.state).execute(&app.db).await?;
    Ok(Json(
        json!({"device_code":device_code,"user_code":user_code,"verification_uri":format!("{}/authorize",app.origin),"verification_uri_complete":format!("{}/authorize?user_code={user_code}",app.origin),"expires_in":600,"interval":5}),
    ))
}
pub async fn request_info(
    State(app): State<App>,
    h: HeaderMap,
    Path(code): Path<String>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, false).await?;
    auth::rate(&app, format!("device-lookup:{user}"), 60, 300).await?;
    let r = sqlx::query("SELECT device_name,return_uri,status FROM device_authorizations WHERE user_code_hash=$1 AND expires_at>now() AND status='pending'")
        .bind(code_hash(&code)?).fetch_optional(&app.db).await?.ok_or_else(||Error::bad("授权请求已失效或已处理，请返回设备重新发起。"))?;
    Ok(Json(
        json!({"device_name":r.get::<String,_>("device_name"),"return_uri":r.get::<String,_>("return_uri"),"scope":SCOPE}),
    ))
}
#[derive(Deserialize)]
pub struct Approval {
    user_code: String,
    approve: bool,
    bundle_id: Option<Uuid>,
}
pub async fn approve(
    State(app): State<App>,
    h: HeaderMap,
    Json(a): Json<Approval>,
) -> Result<Json<Value>, Error> {
    let user = auth::user(&app, &h, true).await?;
    auth::rate(&app, format!("device-approve:{user}"), 30, 300).await?;
    let mut tx = app.db.begin().await?;
    let r = sqlx::query("SELECT device_hash,return_uri,state FROM device_authorizations WHERE user_code_hash=$1 AND expires_at>now() AND status='pending' FOR UPDATE")
        .bind(code_hash(&a.user_code)?).fetch_optional(&mut *tx).await?.ok_or_else(||Error::bad("授权请求已失效或已处理。"))?;
    let bundle = if a.approve {
        let id = a
            .bundle_id
            .ok_or_else(|| Error::bad("请选择要同步的身份。"))?;
        let b = store::get(&app, &mut tx, user, id).await?;
        if b.kind != "bundle" || !b.data["published_revision"].is_string() {
            return Err(Error::bad("请选择已成功发布的身份。"));
        }
        Some(id)
    } else {
        None
    };
    sqlx::query(
        "UPDATE device_authorizations SET status=$2,user_id=$3,bundle_id=$4 WHERE device_hash=$1",
    )
    .bind(r.get::<String, _>("device_hash"))
    .bind(if a.approve { "approved" } else { "denied" })
    .bind(user)
    .bind(bundle)
    .execute(&mut *tx)
    .await?;
    let mut callback = return_uri(&r.get::<String, _>("return_uri"))?;
    callback
        .query_pairs_mut()
        .append_pair("state", &r.get::<String, _>("state"));
    tx.commit().await?;
    Ok(Json(json!({"return_uri":callback.as_str()})))
}
#[derive(Deserialize)]
pub struct TokenRequest {
    client_id: String,
    grant_type: String,
    device_code: String,
}
pub async fn token(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Form(r): Form<TokenRequest>,
) -> Result<Json<Value>, Error> {
    if r.client_id != CLIENT
        || r.grant_type != "urn:ietf:params:oauth:grant-type:device_code"
        || !opaque(&r.device_code)
    {
        return Err(Error::bad("invalid_request"));
    }
    auth::rate(&app, format!("device-poll:{}", peer.ip()), 3000, 300).await?;
    let hash = camofy::digest(&r.device_code);
    let owner: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM device_authorizations WHERE device_hash=$1")
            .bind(&hash)
            .fetch_optional(&app.db)
            .await?
            .flatten();
    let mut tx = app.db.begin().await?;
    // Same lock order as resource deletion: account lock, then authorization row.
    if let Some(user) = owner {
        store::lock(&mut tx, user).await?;
    }
    let row = sqlx::query("SELECT *,expires_at>now() AS valid,next_poll>now() AS too_fast FROM device_authorizations WHERE device_hash=$1 FOR UPDATE")
        .bind(&hash).fetch_optional(&mut *tx).await?.ok_or_else(||Error::bad("expired_token"))?;
    if !row.get::<bool, _>("valid") {
        return Err(Error::bad("expired_token"));
    }
    if row.get::<bool, _>("too_fast") {
        sqlx::query("UPDATE device_authorizations SET poll_interval=poll_interval+5,next_poll=now()+make_interval(secs=>poll_interval+5) WHERE device_hash=$1").bind(&hash).execute(&mut *tx).await?;
        tx.commit().await?;
        return Err(Error::bad("slow_down"));
    }
    let status: String = row.get("status");
    if status == "pending" {
        sqlx::query("UPDATE device_authorizations SET next_poll=now()+make_interval(secs=>poll_interval) WHERE device_hash=$1").bind(hash).execute(&mut *tx).await?;
        tx.commit().await?;
        return Err(Error::bad("authorization_pending"));
    }
    if status == "denied" {
        return Err(Error::bad("access_denied"));
    }
    let user: Uuid = row.get("user_id");
    let bundle: Uuid = row.get("bundle_id");
    if owner != Some(user) {
        return Err(Error::bad("authorization_pending"));
    }
    let b = store::get(&app, &mut tx, user, bundle).await?;
    if b.kind != "bundle" {
        return Err(Error::bad("invalid_grant"));
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM access_tokens WHERE user_id=$1")
        .bind(user)
        .fetch_one(&mut *tx)
        .await?;
    if count >= 100 {
        return Err(Error::new(
            StatusCode::CONFLICT,
            "device credential limit reached",
        ));
    }
    let device = Uuid::new_v4();
    let name: String = row.get("device_name");
    store::put(
        &app,
        &mut tx,
        user,
        &store::Resource {
            id: device,
            kind: "device".into(),
            version: 1,
            data: json!({"name":name,"bundle_id":bundle}),
        },
    )
    .await?;
    let token = secret();
    sqlx::query(
        "INSERT INTO access_tokens(hash,user_id,bundle_id,device_id,label) VALUES($1,$2,$3,$4,$5)",
    )
    .bind(camofy::digest(&token))
    .bind(user)
    .bind(bundle)
    .bind(device)
    .bind(&name)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM device_authorizations WHERE device_hash=$1")
        .bind(hash)
        .execute(&mut *tx)
        .await?;
    store::notify(&mut tx, user).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"access_token":token,"token_type":"Bearer","scope":SCOPE,"cloud_url":app.origin,"device_id":device,"identity_name":b.data["name"]}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_is_local_and_never_an_open_redirect() {
        for url in [
            "http://192.168.1.1:3000/bind/complete",
            "http://127.0.0.1:3333/bind/complete",
            "http://[fd00::1]:3000/bind/complete",
        ] {
            assert!(return_uri(url).is_ok());
        }
        for url in [
            "https://evil.example/bind/complete",
            "http://8.8.8.8/bind/complete",
            "http://169.254.169.254/bind/complete",
            "http://192.168.1.1/admin",
            "http://user@192.168.1.1/bind/complete",
            "http://192.168.1.1/bind/complete?next=evil",
            "javascript:alert(1)",
        ] {
            assert!(return_uri(url).is_err(), "{url}");
        }
    }
}
