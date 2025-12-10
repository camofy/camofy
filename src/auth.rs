use axum::{
    body::Body,
    http::Request,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::app::{app_state, AppState};
use crate::{ApiResponse, AppConfig, get_app_config_snapshot, with_app_config_mut};

#[derive(Serialize)]
pub struct SettingsDto {
    pub password_set: bool,
    #[serde(default)]
    pub subscription_auto_update: Option<crate::ScheduledTaskConfig>,
    #[serde(default)]
    pub geoip_auto_update: Option<crate::ScheduledTaskConfig>,
}

#[derive(Deserialize)]
pub struct UpdateSettingsRequest {
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub subscription_auto_update: Option<crate::ScheduledTaskConfig>,
    #[serde(default)]
    pub geoip_auto_update: Option<crate::ScheduledTaskConfig>,
}

#[derive(Deserialize)]
pub struct AuthLoginRequest {
    pub password: String,
}

#[derive(Serialize)]
pub struct AuthLoginResponse {
    pub token: String,
    pub expires_at: u64,
}

async fn validate_token(state: &AppState, token: &str) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut guard = state.auth_tokens.lock().await;
    guard.retain(|s| s.expires_at > now);
    guard.iter().any(|s| s.token == token)
}

pub async fn api_auth_middleware(req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path();

    // 不进行认证的接口：
    // - /health
    // - /auth/login
    let is_public = path == "/health" || path == "/auth/login";

    let state = app_state();
    let requires_auth = get_app_config_snapshot()
        .panel_password_hash
        .is_some();

    if !requires_auth {
        // 未设置密码时，所有接口无需认证
        return next.run(req).await;
    }

    if is_public {
        return next.run(req).await;
    }

    // 先尝试从 Header 中获取 Token（常规 REST 请求）。
    let mut token = req
        .headers()
        .get("X-Auth-Token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // 对于 WebSocket 等无法自定义 Header 的场景，从查询参数 token 中兜底获取。
    if token.is_none() {
        if let Some(query) = req.uri().query() {
            for pair in query.split('&') {
                if let Some(rest) = pair.strip_prefix("token=") {
                    // token 目前是 UUID，不涉及复杂编码，这里直接使用原始值。
                    token = Some(rest.to_string());
                    break;
                }
            }
        }
    }

    if let Some(token) = token {
        if validate_token(state, &token).await {
            return next.run(req).await;
        }
    }

    let body = Json(ApiResponse::<serde_json::Value> {
        code: "unauthorized".to_string(),
        message: "authentication required".to_string(),
        data: None,
    });
    body.into_response()
}

pub async fn get_settings() -> Json<ApiResponse<SettingsDto>> {
    let cfg = get_app_config_snapshot();
    let data = SettingsDto {
        password_set: cfg.panel_password_hash.is_some(),
        subscription_auto_update: cfg.subscription_auto_update,
        geoip_auto_update: cfg.geoip_auto_update,
    };
    Json(ApiResponse {
        code: "ok".to_string(),
        message: "success".to_string(),
        data: Some(data),
    })
}

pub async fn update_settings(
    Json(body): Json<UpdateSettingsRequest>,
) -> Json<ApiResponse<SettingsDto>> {
    use argon2::{Argon2, password_hash::{PasswordHasher, SaltString}};
    use rand_core::OsRng;

    // 先在锁外完成密码相关校验与哈希计算，避免在持有写锁时做重计算或早返回。
    let new_password_hash = if let Some(password) = body.password.as_deref() {
        let trimmed = password.trim();
        if trimmed.is_empty() {
            return Json(ApiResponse {
                code: "settings_invalid_password".to_string(),
                message: "password cannot be empty".to_string(),
                data: None,
            });
        }

        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();

        let hash = match argon2.hash_password(trimmed.as_bytes(), &salt) {
            Ok(h) => h.to_string(),
            Err(err) => {
                let msg = format!("failed to hash password: {err}");
                tracing::error!("{msg}");
                return Json(ApiResponse {
                    code: "settings_hash_failed".to_string(),
                    message: msg,
                    data: None,
                });
            }
        };

        Some(hash)
    } else {
        None
    };

    let sub_task = body.subscription_auto_update.clone();
    let geoip_task = body.geoip_auto_update.clone();

    let result = with_app_config_mut(|config: &mut AppConfig| {
        if let Some(hash) = new_password_hash.as_ref() {
            config.panel_password_hash = Some(hash.clone());
        }

        if let Some(task) = sub_task {
            config.subscription_auto_update = Some(task);
        }
        if let Some(task) = geoip_task {
            config.geoip_auto_update = Some(task);
        }

        SettingsDto {
            password_set: config.panel_password_hash.is_some(),
            subscription_auto_update: config.subscription_auto_update.clone(),
            geoip_auto_update: config.geoip_auto_update.clone(),
        }
    });

    match result {
        Ok(dto) => Json(ApiResponse {
            code: "ok".to_string(),
            message: "settings_updated".to_string(),
            data: Some(dto),
        }),
        Err(err) => {
            tracing::error!("{err}");
            Json(ApiResponse {
                code: "config_save_failed".to_string(),
                message: err,
                data: None,
            })
        }
    }
}

pub async fn auth_login(
    Json(body): Json<AuthLoginRequest>,
) -> Json<ApiResponse<AuthLoginResponse>> {
    use argon2::{Argon2, password_hash::{PasswordHash, PasswordVerifier}};

    let state = app_state();

    let config: AppConfig = get_app_config_snapshot();

    let hash_str = match config.panel_password_hash.as_deref() {
        Some(h) => h.to_string(),
        None => {
            return Json(ApiResponse {
                code: "auth_password_not_set".to_string(),
                message: "panel password is not set".to_string(),
                data: None,
            });
        }
    };

    let parsed_hash = match PasswordHash::new(&hash_str) {
        Ok(h) => h,
        Err(err) => {
            let msg = format!("invalid stored password hash: {err}");
            tracing::error!("{msg}");
            // 清除损坏的密码，避免死锁状态
            let _ = with_app_config_mut(|cfg: &mut AppConfig| {
                cfg.panel_password_hash = None;
            });
            return Json(ApiResponse {
                code: "auth_invalid_password_store".to_string(),
                message: msg,
                data: None,
            });
        }
    };

    let argon2 = Argon2::default();
    if let Err(err) = argon2.verify_password(body.password.as_bytes(), &parsed_hash) {
        tracing::warn!("auth failed: {err}");
        return Json(ApiResponse {
            code: "auth_invalid_password".to_string(),
            message: "invalid password".to_string(),
            data: None,
        });
    }

    // 密码验证成功，生成会话 Token
    let token = uuid::Uuid::new_v4().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let ttl_secs: u64 = 8 * 60 * 60;
    let expires_at = now + ttl_secs;

    {
        let mut guard = state.auth_tokens.lock().await;
        // 清理过期 Token
        guard.retain(|s| s.expires_at > now);
        guard.push(crate::app::AuthSession { token: token.clone(), expires_at });
    }

    let data = AuthLoginResponse { token, expires_at };

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "login success".to_string(),
        data: Some(data),
    })
}
