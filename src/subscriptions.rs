use std::path::PathBuf;

use axum::{extract::Path, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::app_state;
use crate::app::current_timestamp;
use crate::{ApiResponse, AppConfig, ProfileMeta, ProfileType};
use crate::{load_app_config, save_app_config};

#[derive(Serialize)]
pub struct SubscriptionDto {
    pub id: String,
    pub name: String,
    pub url: String,
    pub is_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fetch_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fetch_status: Option<String>,
}

#[derive(Serialize)]
pub struct SubscriptionListResponse {
    pub subscriptions: Vec<SubscriptionDto>,
}

#[derive(Deserialize)]
pub struct CreateSubscriptionRequest {
    pub name: String,
    pub url: String,
}

#[derive(Deserialize)]
pub struct UpdateSubscriptionRequest {
    pub name: String,
    pub url: String,
}

fn subscription_dir(root: &PathBuf, id: &str) -> PathBuf {
    let mut path = root.clone();
    path.push("config");
    path.push("subscriptions");
    path.push(id);
    path
}

fn to_subscription_dto(config: &AppConfig, profile: &ProfileMeta) -> Option<SubscriptionDto> {
    if !matches!(profile.profile_type, ProfileType::Remote) {
        return None;
    }

    let active_id = config.active_subscription_id.as_deref();
    let id = profile.id.clone();
    let name = profile.name.clone();
    let url = profile.url.clone().unwrap_or_default();

    Some(SubscriptionDto {
        id,
        name,
        url,
        is_active: active_id == Some(profile.id.as_str()),
        last_fetch_time: profile.last_fetch_time.clone(),
        last_fetch_status: profile.last_fetch_status.clone(),
    })
}

pub async fn list_subscriptions() -> Json<ApiResponse<SubscriptionListResponse>> {
    let state = app_state();

    match load_app_config(&state.data_root) {
        Ok(config) => {
            let subscriptions = config
                .profiles
                .iter()
                .filter_map(|p| to_subscription_dto(&config, p))
                .collect();

            Json(ApiResponse {
                code: "ok".to_string(),
                message: "success".to_string(),
                data: Some(SubscriptionListResponse { subscriptions }),
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

pub async fn create_subscription(
    Json(body): Json<CreateSubscriptionRequest>,
) -> Json<ApiResponse<SubscriptionDto>> {
    let state = app_state();

    let mut config = match load_app_config(&state.data_root) {
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

    let id = Uuid::new_v4().to_string();
    let path = format!("subscriptions/{id}/subscription.yaml");
    let profile = ProfileMeta {
        id: id.clone(),
        name: body.name,
        profile_type: ProfileType::Remote,
        path,
        url: Some(body.url),
        last_fetch_time: None,
        last_fetch_status: None,
        last_modified_time: None,
    };

    if config.active_subscription_id.is_none() {
        config.active_subscription_id = Some(id.clone());
    }

    config.profiles.push(profile.clone());

    if let Err(err) = save_app_config(&state.data_root, &config) {
        tracing::error!("{err}");
        return Json(ApiResponse {
            code: "config_save_failed".to_string(),
            message: err,
            data: None,
        });
    }

    let dto = match to_subscription_dto(&config, &profile) {
        Some(dto) => dto,
        None => {
            return Json(ApiResponse {
                code: "internal_error".to_string(),
                message: "failed to build subscription DTO".to_string(),
                data: None,
            })
        }
    };

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "created".to_string(),
        data: Some(dto),
    })
}

pub async fn update_subscription(
    Path(id): Path<String>,
    Json(body): Json<UpdateSubscriptionRequest>,
) -> Json<ApiResponse<SubscriptionDto>> {
    let state = app_state();

    let mut config = match load_app_config(&state.data_root) {
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

    let Some(profile) = config
        .profiles
        .iter_mut()
        .find(|p| matches!(p.profile_type, ProfileType::Remote) && p.id == id)
    else {
        return Json(ApiResponse::<SubscriptionDto> {
            code: "subscription_not_found".to_string(),
            message: "subscription not found".to_string(),
            data: None,
        });
    };

    profile.name = body.name;
    profile.url = Some(body.url);

    let updated = profile.clone();

    if let Err(err) = save_app_config(&state.data_root, &config) {
        tracing::error!("{err}");
        return Json(ApiResponse {
            code: "config_save_failed".to_string(),
            message: err,
            data: None,
        });
    }

    let dto = match to_subscription_dto(&config, &updated) {
        Some(dto) => dto,
        None => {
            return Json(ApiResponse {
                code: "internal_error".to_string(),
                message: "failed to build subscription DTO".to_string(),
                data: None,
            })
        }
    };

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "updated".to_string(),
        data: Some(dto),
    })
}

pub async fn delete_subscription(
    Path(id): Path<String>,
) -> Json<ApiResponse<serde_json::Value>> {
    use std::fs;

    let state = app_state();

    let mut config = match load_app_config(&state.data_root) {
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

    let original_len = config.profiles.len();
    config
        .profiles
        .retain(|p| !(matches!(p.profile_type, ProfileType::Remote) && p.id == id));

    if config.profiles.len() == original_len {
        return Json(ApiResponse {
            code: "subscription_not_found".to_string(),
            message: "subscription not found".to_string(),
            data: None,
        });
    }

    if config
        .active_subscription_id
        .as_ref()
        .is_some_and(|active| active == &id)
    {
        config.active_subscription_id = config
            .profiles
            .iter()
            .find(|p| matches!(p.profile_type, ProfileType::Remote))
            .map(|p| p.id.clone());
    }

    if let Err(err) = save_app_config(&state.data_root, &config) {
        tracing::error!("{err}");
        return Json(ApiResponse {
            code: "config_save_failed".to_string(),
            message: err,
            data: None,
        });
    }

    let dir = subscription_dir(&state.data_root, &id);
    if let Err(err) = fs::remove_dir_all(&dir) {
        if err.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("failed to remove subscription directory {}: {err}", dir.display());
        }
    }

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "deleted".to_string(),
        data: Some(serde_json::json!({})),
    })
}

pub async fn activate_subscription(
    Path(id): Path<String>,
) -> Json<ApiResponse<serde_json::Value>> {
    let state = app_state();

    let mut config = match load_app_config(&state.data_root) {
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

    if !config
        .profiles
        .iter()
        .any(|p| matches!(p.profile_type, ProfileType::Remote) && p.id == id)
    {
        return Json(ApiResponse {
            code: "subscription_not_found".to_string(),
            message: "subscription not found".to_string(),
            data: None,
        });
    }

    config.active_subscription_id = Some(id);

    if let Err(err) = save_app_config(&state.data_root, &config) {
        tracing::error!("{err}");
        return Json(ApiResponse {
            code: "config_save_failed".to_string(),
            message: err,
            data: None,
        });
    }

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "activated".to_string(),
        data: Some(serde_json::json!({})),
    })
}

pub async fn fetch_subscription(
    Path(id): Path<String>,
) -> Json<ApiResponse<serde_json::Value>> {
    use std::fs;

    let state = app_state();

    let mut config = match load_app_config(&state.data_root) {
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

    let Some(profile) = config
        .profiles
        .iter_mut()
        .find(|p| matches!(p.profile_type, ProfileType::Remote) && p.id == id)
    else {
        return Json(ApiResponse {
            code: "subscription_not_found".to_string(),
            message: "subscription not found".to_string(),
            data: None,
        });
    };

    let Some(url) = profile.url.clone() else {
        return Json(ApiResponse {
            code: "subscription_url_missing".to_string(),
            message: "subscription url is missing".to_string(),
            data: None,
        });
    };

    let content = match state.http_client.get(&url).send().await {
        Ok(resp) => match resp.error_for_status() {
            Ok(ok) => match ok.text().await {
                Ok(text) => text,
                Err(err) => {
                    let msg = format!("failed to read response body: {err}");
                    tracing::error!("{msg}");
                    profile.last_fetch_status = Some("body_read_failed".to_string());
                    if let Err(save_err) = save_app_config(&state.data_root, &config) {
                        tracing::error!("{save_err}");
                    }
                    return Json(ApiResponse {
                        code: "subscription_fetch_failed".to_string(),
                        message: msg,
                        data: None,
                    });
                }
            },
            Err(err) => {
                let msg = format!("request failed: {err}");
                tracing::error!("{msg}");
                profile.last_fetch_status = Some("request_failed".to_string());
                if let Err(save_err) = save_app_config(&state.data_root, &config) {
                    tracing::error!("{save_err}");
                }
                return Json(ApiResponse {
                    code: "subscription_fetch_failed".to_string(),
                    message: msg,
                    data: None,
                });
            }
        },
        Err(err) => {
            let msg = format!("failed to send request: {err}");
            tracing::error!("{msg}");
            profile.last_fetch_status = Some("request_failed".to_string());
            if let Err(save_err) = save_app_config(&state.data_root, &config) {
                tracing::error!("{save_err}");
            }
            return Json(ApiResponse {
                code: "subscription_fetch_failed".to_string(),
                message: msg,
                data: None,
            });
        }
    };

    let dir = subscription_dir(&state.data_root, &id);
    if let Err(err) = fs::create_dir_all(&dir) {
        let msg = format!(
            "failed to create subscription directory {}: {err}",
            dir.display()
        );
        tracing::error!("{msg}");
        profile.last_fetch_status = Some("write_failed".to_string());
        if let Err(save_err) = save_app_config(&state.data_root, &config) {
            tracing::error!("{save_err}");
        }
        return Json(ApiResponse {
            code: "subscription_save_failed".to_string(),
            message: msg,
            data: None,
        });
    }

    let subscription_path = dir.join("subscription.yaml");

    if let Err(err) = fs::write(&subscription_path, &content) {
        let msg = format!("failed to write {}: {err}", subscription_path.display());
        tracing::error!("{msg}");
        profile.last_fetch_status = Some("write_failed".to_string());
        if let Err(save_err) = save_app_config(&state.data_root, &config) {
            tracing::error!("{save_err}");
        }
        return Json(ApiResponse {
            code: "subscription_save_failed".to_string(),
            message: msg,
            data: None,
        });
    }

    profile.last_fetch_status = Some("ok".to_string());
    profile.last_fetch_time = Some(current_timestamp());

    if let Err(err) = save_app_config(&state.data_root, &config) {
        tracing::error!("{err}");
        return Json(ApiResponse {
            code: "config_save_failed".to_string(),
            message: err,
            data: None,
        });
    }

    // 拉取订阅成功后尝试生成 merged.yaml
    if let Err(err) = crate::user_profiles::generate_merged_config(&state.data_root) {
        tracing::error!("failed to generate merged config after fetch: {err}");
        return Json(ApiResponse {
            code: "config_merge_failed".to_string(),
            message: err,
            data: None,
        });
    }

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "fetched".to_string(),
        data: Some(serde_json::json!({})),
    })
}

/// 自动更新当前“活跃订阅”的订阅内容。
///
/// - 若未设置活跃订阅，则返回 Err("skipped:...")，由调度器记录为跳过状态。
/// - 其余错误则用于调度器记录为失败状态。
pub async fn auto_update_subscriptions() -> Result<(), String> {
    use axum::extract::Path;

    let state = app_state();

    let config = load_app_config(&state.data_root)
        .map_err(|err| format!("failed to load app config for auto subscription update: {err}"))?;

    let Some(active_id) = config.active_subscription_id.clone() else {
        return Err("skipped:no_active_subscription".to_string());
    };

    let Json(resp) = fetch_subscription(Path(active_id)).await;
    if resp.code == "ok" {
        Ok(())
    } else {
        Err(format!(
            "subscription_auto_update_failed: {}",
            resp.message
        ))
    }
}
