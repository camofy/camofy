use axum::{
    body::Body,
    http::Request,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::app::{app_state, AppState};
use crate::{ApiResponse, AppConfig};

#[derive(Serialize)]
pub struct SettingsDto {
    pub password_set: bool,
}

#[derive(Deserialize)]
pub struct UpdateSettingsRequest {
    #[serde(default)]
    pub password: Option<String>,
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
    let requires_auth = match crate::load_app_config(&state.data_root) {
        Ok(cfg) => cfg.panel_password_hash.is_some(),
        Err(err) => {
            tracing::error!("failed to load app config in auth middleware: {err}");
            false
        }
    };

    if !requires_auth {
        // 未设置密码时，所有接口无需认证
        return next.run(req).await;
    }

    if is_public {
        return next.run(req).await;
    }

    let token = req
        .headers()
        .get("X-Auth-Token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

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
    let state = app_state();

    match crate::load_app_config(&state.data_root) {
        Ok(cfg) => {
            let data = SettingsDto {
                password_set: cfg.panel_password_hash.is_some(),
            };
            Json(ApiResponse {
                code: "ok".to_string(),
                message: "success".to_string(),
                data: Some(data),
            })
        }
        Err(err) => {
            tracing::error!("{err}");
            Json(ApiResponse {
                code: "config_load_failed".to_string(),
                message: err,
                data: None,
            })
        }
    }
}

pub async fn update_settings(
    Json(body): Json<UpdateSettingsRequest>,
) -> Json<ApiResponse<SettingsDto>> {
    use argon2::{Argon2, password_hash::{PasswordHasher, SaltString}};
    use rand_core::OsRng;

    let state = app_state();

    let mut config: AppConfig = match crate::load_app_config(&state.data_root) {
        Ok(cfg) => cfg,
        Err(err) => {
            tracing::error!("{err}");
            return Json(ApiResponse {
                code: "config_load_failed".to_string(),
                message: err,
                data: None,
            });
        }
    };

    if let Some(password) = body.password.as_deref() {
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

        config.panel_password_hash = Some(hash);
    }

    if let Err(err) = crate::save_app_config(&state.data_root, &config) {
        tracing::error!("{err}");
        return Json(ApiResponse {
            code: "config_save_failed".to_string(),
            message: err,
            data: None,
        });
    }

    let dto = SettingsDto {
        password_set: config.panel_password_hash.is_some(),
    };

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "settings_updated".to_string(),
        data: Some(dto),
    })
}

pub async fn auth_login(
    Json(body): Json<AuthLoginRequest>,
) -> Json<ApiResponse<AuthLoginResponse>> {
    use argon2::{Argon2, password_hash::{PasswordHash, PasswordVerifier}};

    let state = app_state();

    let mut config: AppConfig = match crate::load_app_config(&state.data_root) {
        Ok(cfg) => cfg,
        Err(err) => {
            tracing::error!("{err}");
            return Json(ApiResponse {
                code: "config_load_failed".to_string(),
                message: err,
                data: None,
            });
        }
    };

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
            config.panel_password_hash = None;
            if let Err(save_err) = crate::save_app_config(&state.data_root, &config) {
                tracing::error!("{save_err}");
            }
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
