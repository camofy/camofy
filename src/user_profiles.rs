use std::path::PathBuf;

use axum::{Json, extract::Path};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::{app_state, current_timestamp};
use crate::{ApiResponse, AppConfig, ProfileMeta, ProfileType};
use crate::{load_app_config, save_app_config};

#[derive(Serialize)]
pub struct UserProfileSummary {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified_time: Option<String>,
}

#[derive(Serialize)]
pub struct UserProfileListResponse {
    pub user_profiles: Vec<UserProfileSummary>,
}

#[derive(Serialize)]
pub struct UserProfileDetail {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified_time: Option<String>,
    pub content: String,
}

#[derive(Deserialize)]
pub struct CreateUserProfileRequest {
    pub name: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Deserialize)]
pub struct UpdateUserProfileRequest {
    pub name: String,
    pub content: String,
}

fn user_profile_path(root: &PathBuf, id: &str) -> PathBuf {
    let mut path = root.clone();
    path.push("config");
    path.push("user-profiles");
    path.push(format!("{id}.yaml"));
    path
}

fn profile_file_path(root: &PathBuf, profile: &ProfileMeta) -> PathBuf {
    let mut path = root.clone();
    path.push("config");
    path.push(&profile.path);
    path
}

fn to_user_profile_summary(
    config: &AppConfig,
    profile: &ProfileMeta,
) -> Option<UserProfileSummary> {
    if !matches!(profile.profile_type, ProfileType::User) {
        return None;
    }

    let active_id = config.active_user_profile_id.as_deref();

    Some(UserProfileSummary {
        id: profile.id.clone(),
        name: profile.name.clone(),
        is_active: active_id == Some(profile.id.as_str()),
        last_modified_time: profile.last_modified_time.clone(),
    })
}

pub async fn get_user_profile(Path(id): Path<String>) -> Json<ApiResponse<UserProfileDetail>> {
    use std::fs;

    let state = app_state();

    // 从全局配置中获取 profile 元数据和活跃状态
    let (profile_meta, active_id) = {
        let guard = state
            .app_config
            .read()
            .expect("app config rwlock poisoned");
        let config: &AppConfig = &guard;

        let Some(profile) = config
            .profiles
            .iter()
            .find(|p| matches!(p.profile_type, ProfileType::User) && p.id == id)
        else {
            return Json(ApiResponse {
                code: "user_profile_not_found".to_string(),
                message: "user profile not found".to_string(),
                data: None,
            });
        };

        (profile.clone(), config.active_user_profile_id.clone())
    };

   let path = profile_file_path(&state.data_root, &profile_meta);
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(err) => {
            let msg = format!("failed to read user profile file {}: {err}", path.display());
            tracing::error!("{msg}");
            return Json(ApiResponse {
                code: "user_profile_read_failed".to_string(),
                message: msg,
                data: None,
            });
        }
    };

    let active_id = active_id.as_deref();

    let detail = UserProfileDetail {
        id: profile_meta.id.clone(),
        name: profile_meta.name.clone(),
        is_active: active_id == Some(profile_meta.id.as_str()),
        last_modified_time: profile_meta.last_modified_time.clone(),
        content,
    };

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "success".to_string(),
        data: Some(detail),
    })
}

pub async fn list_user_profiles() -> Json<ApiResponse<UserProfileListResponse>> {
    let state = app_state();

    let guard = state
        .app_config
        .read()
        .expect("app config rwlock poisoned");
    let config: &AppConfig = &guard;

    let user_profiles = config
        .profiles
        .iter()
        .filter_map(|p| to_user_profile_summary(config, p))
        .collect();

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "success".to_string(),
        data: Some(UserProfileListResponse { user_profiles }),
    })
}

pub async fn create_user_profile(
    Json(body): Json<CreateUserProfileRequest>,
) -> Json<ApiResponse<UserProfileSummary>> {
    let state = app_state();

    // 如果提供了内容且非空，先校验 YAML 格式；空内容视为一个空配置
    let trimmed = body.content.trim();
    let content_to_write = if trimmed.is_empty() {
        "# empty user profile\n".to_string()
    } else {
        if let Err(err) = serde_yaml::from_str::<serde_yaml::Value>(trimmed) {
            let msg = format!("invalid user profile yaml: {err}");
            tracing::error!("{msg}");
            return Json(ApiResponse {
                code: "user_profile_invalid_yaml".to_string(),
                message: msg,
                data: None,
            });
        }
        body.content
    };

    // 在全局配置中创建 profile 元数据并持久化
    let (profile, summary) = {
        let mut guard = state
            .app_config
            .write()
            .expect("app config rwlock poisoned");
        let config: &mut AppConfig = &mut guard;

        let id = Uuid::new_v4().to_string();
        let path = format!("user-profiles/{id}.yaml");

        let profile = ProfileMeta {
            id: id.clone(),
            name: body.name,
            profile_type: ProfileType::User,
            path,
            url: None,
            last_fetch_time: None,
            last_fetch_status: None,
            last_modified_time: Some(current_timestamp()),
        };

        // 如果当前没有活跃用户 profile，则将新建的设为活跃
        if config.active_user_profile_id.is_none() {
            config.active_user_profile_id = Some(id.clone());
        }

        config.profiles.push(profile.clone());

        if let Err(err) = save_app_config(&state.data_root, config) {
            tracing::error!("{err}");
            return Json(ApiResponse {
                code: "config_save_failed".to_string(),
                message: err,
                data: None,
            });
        }

        let summary = match to_user_profile_summary(config, &profile) {
            Some(s) => s,
            None => {
                return Json(ApiResponse {
                    code: "internal_error".to_string(),
                    message: "failed to build user profile summary".to_string(),
                    data: None,
                });
            }
        };

        (profile, summary)
    };

    // 写入用户 profile 文件
    let path = profile_file_path(&state.data_root, &profile);
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            let msg = format!(
                "failed to create user profile dir {}: {err}",
                parent.display()
            );
            tracing::error!("{msg}");
            return Json(ApiResponse {
                code: "user_profile_write_failed".to_string(),
                message: msg,
                data: None,
            });
        }
    }
    if let Err(err) = std::fs::write(&path, content_to_write) {
        let msg = format!("failed to write user profile {}: {err}", path.display());
        tracing::error!("{msg}");
        return Json(ApiResponse {
            code: "user_profile_write_failed".to_string(),
            message: msg,
            data: None,
        });
    }

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "created".to_string(),
        data: Some(summary),
    })
}

pub async fn update_user_profile(
    Path(id): Path<String>,
    Json(body): Json<UpdateUserProfileRequest>,
) -> Json<ApiResponse<UserProfileDetail>> {
    use std::fs;

    let state = app_state();

    // 先校验内容是否合法 YAML
    let trimmed = body.content.trim();
    let content_to_write = if trimmed.is_empty() {
        "# empty user profile\n".to_string()
    } else {
        if let Err(err) = serde_yaml::from_str::<serde_yaml::Value>(trimmed) {
            let msg = format!("invalid user profile yaml: {err}");
            tracing::error!("{msg}");
            return Json(ApiResponse {
                code: "user_profile_invalid_yaml".to_string(),
                message: msg,
                data: None,
            });
        }
        body.content
    };

    // 更新全局配置中的 profile 元数据
    let (updated_profile, is_active) = {
        let mut guard = state
            .app_config
            .write()
            .expect("app config rwlock poisoned");
        let config: &mut AppConfig = &mut guard;

        let Some(profile) = config
            .profiles
            .iter_mut()
            .find(|p| matches!(p.profile_type, ProfileType::User) && p.id == id)
        else {
            return Json(ApiResponse {
                code: "user_profile_not_found".to_string(),
                message: "user profile not found".to_string(),
                data: None,
            });
        };

        profile.name = body.name;
        profile.last_modified_time = Some(current_timestamp());

        let updated_profile = profile.clone();
        let is_active = config
            .active_user_profile_id
            .as_deref()
            .is_some_and(|active| active == updated_profile.id.as_str());

        if let Err(err) = save_app_config(&state.data_root, config) {
            tracing::error!("{err}");
            return Json(ApiResponse {
                code: "config_save_failed".to_string(),
                message: err,
                data: None,
            });
        }

        (updated_profile, is_active)
    };

    let path = profile_file_path(&state.data_root, &updated_profile);
    if let Some(parent) = path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            let msg = format!(
                "failed to create user profile dir {}: {err}",
                parent.display()
            );
            tracing::error!("{msg}");
            return Json(ApiResponse {
                code: "user_profile_write_failed".to_string(),
                message: msg,
                data: None,
            });
        }
    }

    if let Err(err) = fs::write(&path, &content_to_write) {
        let msg = format!("failed to write user profile {}: {err}", path.display());
        tracing::error!("{msg}");
        return Json(ApiResponse {
            code: "user_profile_write_failed".to_string(),
            message: msg,
            data: None,
        });
    }

    // 更新用户 profile 成功后尝试重新生成 merged.yaml
    if let Err(err) = generate_merged_config(&state.data_root) {
        tracing::error!("failed to generate merged config after user profile update: {err}");
        return Json(ApiResponse {
            code: "config_merge_failed".to_string(),
            message: err,
            data: None,
        });
    }

    let detail = UserProfileDetail {
        id: updated_profile.id.clone(),
        name: updated_profile.name.clone(),
        is_active,
        last_modified_time: updated_profile.last_modified_time.clone(),
        content: content_to_write,
    };

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "updated".to_string(),
        data: Some(detail),
    })
}

pub async fn delete_user_profile(Path(id): Path<String>) -> Json<ApiResponse<serde_json::Value>> {
    let state = app_state();

    use std::fs;

    let mut removed = false;

    {
        let mut guard = state
            .app_config
            .write()
            .expect("app config rwlock poisoned");
        let config: &mut AppConfig = &mut guard;

        let original_len = config.profiles.len();
        config
            .profiles
            .retain(|p| !(matches!(p.profile_type, ProfileType::User) && p.id == id));

        if config.profiles.len() == original_len {
            // not found
        } else {
            removed = true;

            if config
                .active_user_profile_id
                .as_ref()
                .is_some_and(|active| active == &id)
            {
                config.active_user_profile_id = None;
            }

            if let Err(err) = save_app_config(&state.data_root, config) {
                tracing::error!("{err}");
                return Json(ApiResponse {
                    code: "config_save_failed".to_string(),
                    message: err,
                    data: None,
                });
            }
        }
    }

    if !removed {
        return Json(ApiResponse {
            code: "user_profile_not_found".to_string(),
            message: "user profile not found".to_string(),
            data: None,
        });
    }

    let path = user_profile_path(&state.data_root, &id);
    if let Err(err) = fs::remove_file(&path) {
        if err.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(
                "failed to remove user profile file {}: {err}",
                path.display()
            );
        }
    }

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "deleted".to_string(),
        data: Some(serde_json::json!({})),
    })
}

pub async fn activate_user_profile(Path(id): Path<String>) -> Json<ApiResponse<serde_json::Value>> {
    let state = app_state();

    let mut found = false;

    {
        let mut guard = state
            .app_config
            .write()
            .expect("app config rwlock poisoned");
        let config: &mut AppConfig = &mut guard;

        if !config
            .profiles
            .iter()
            .any(|p| matches!(p.profile_type, ProfileType::User) && p.id == id)
        {
            // not found
        } else {
            found = true;
            config.active_user_profile_id = Some(id.clone());

            if let Err(err) = save_app_config(&state.data_root, config) {
                tracing::error!("{err}");
                return Json(ApiResponse {
                    code: "config_save_failed".to_string(),
                    message: err,
                    data: None,
                });
            }
        }
    }

    if !found {
        return Json(ApiResponse {
            code: "user_profile_not_found".to_string(),
            message: "user profile not found".to_string(),
            data: None,
        });
    }

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "activated".to_string(),
        data: Some(serde_json::json!({})),
    })
}

fn load_yaml_file(path: &std::path::Path) -> Result<serde_yaml::Value, String> {
    use std::fs;

    let content = fs::read_to_string(path)
        .map_err(|err| format!("failed to read yaml file {}: {err}", path.display()))?;

    if content.trim().is_empty() {
        return Ok(serde_yaml::Value::Null);
    }

    serde_yaml::from_str::<serde_yaml::Value>(&content)
        .map_err(|err| format!("failed to parse yaml at {}: {err}", path.display()))
}

fn merge_yaml_configs(
    remote: Option<&serde_yaml::Value>,
    user: Option<&serde_yaml::Value>,
) -> Result<serde_yaml::Value, String> {
    use serde_yaml::{Mapping, Sequence, Value};
    fn to_mapping(value: Option<&Value>, label: &str) -> Result<Mapping, String> {
        match value {
            None => Ok(Mapping::new()),
            Some(Value::Mapping(m)) => Ok(m.clone()),
            Some(Value::Null) => Ok(Mapping::new()),
            Some(other) => Err(format!("{label} root must be mapping, got {other:?}")),
        }
    }

    fn deep_merge_maps(target: &mut Mapping, source: &Mapping) {
        for (k, v) in source {
            match (target.get_mut(k), v) {
                (Some(Value::Mapping(dst)), Value::Mapping(src)) => {
                    deep_merge_maps(dst, src);
                }
                _ => {
                    target.insert(k.clone(), v.clone());
                }
            }
        }
    }

    fn merge_sequence_field(
        field: &str,
        remote: &Mapping,
        user: &Mapping,
        prepend: Option<&Value>,
        append: Option<&Value>,
    ) -> Result<Option<Sequence>, String> {
        let key = Value::String(field.to_string());

        let base_val = user.get(&key).or_else(|| remote.get(&key));
        let base_seq = match base_val {
            Some(Value::Sequence(seq)) => seq.clone(),
            Some(Value::Null) | None => Sequence::new(),
            Some(other) => {
                return Err(format!(
                    "field '{field}' must be a sequence when present, got {other:?}"
                ));
            }
        };

        let mut result = Sequence::new();

        if let Some(v) = prepend {
            match v {
                Value::Sequence(seq) => result.extend(seq.clone()),
                Value::Null => {}
                other => {
                    return Err(format!(
                        "field 'prepend-{field}' must be a sequence when present, got {other:?}"
                    ));
                }
            }
        }

        result.extend(base_seq);

        if let Some(v) = append {
            match v {
                Value::Sequence(seq) => result.extend(seq.clone()),
                Value::Null => {}
                other => {
                    return Err(format!(
                        "field 'append-{field}' must be a sequence when present, got {other:?}"
                    ));
                }
            }
        }

        if result.is_empty() && base_val.is_none() && prepend.is_none() && append.is_none() {
            Ok(None)
        } else {
            Ok(Some(result))
        }
    }

    let remote_map = to_mapping(remote, "remote profile")?;
    let user_map = to_mapping(user, "user profile")?;

    let mut result = remote_map.clone();

    for (k, v) in &user_map {
        let key_str = k.as_str();

        // 特殊字段仅作为合并指令使用，不直接写入结果
        if let Some(name) = key_str {
            if matches!(
                name,
                "prepend-rules"
                    | "append-rules"
                    | "prepend-proxies"
                    | "append-proxies"
                    | "prepend-proxy-groups"
                    | "append-proxy-groups"
            ) {
                continue;
            }
        }

        match (result.get_mut(k), v) {
            // 深度合并对象
            (Some(Value::Mapping(dst)), Value::Mapping(src))
                if key_str != Some("rules") && key_str != Some("proxies") =>
            {
                deep_merge_maps(dst, src);
            }
            // 其他情况直接覆盖（含列表）
            _ => {
                result.insert(k.clone(), v.clone());
            }
        }
    }

    let mut final_map = result;

    let key_prepend_rules = Value::String("prepend-rules".into());
    let key_append_rules = Value::String("append-rules".into());
    let key_prepend_proxies = Value::String("prepend-proxies".into());
    let key_append_proxies = Value::String("append-proxies".into());
    let key_prepend_proxy_groups = Value::String("prepend-proxy-groups".into());
    let key_append_proxy_groups = Value::String("append-proxy-groups".into());

    // 处理 rules
    let prepend_rules = user_map.get(&key_prepend_rules);
    let append_rules = user_map.get(&key_append_rules);
    if prepend_rules.is_some() || append_rules.is_some() {
        if let Some(seq) =
            merge_sequence_field("rules", &remote_map, &user_map, prepend_rules, append_rules)?
        {
            final_map.insert(Value::String("rules".into()), Value::Sequence(seq));
        }
    }

    // 处理 proxies
    let prepend_proxies = user_map.get(&key_prepend_proxies);
    let append_proxies = user_map.get(&key_append_proxies);
    if prepend_proxies.is_some() || append_proxies.is_some() {
        if let Some(seq) = merge_sequence_field(
            "proxies",
            &remote_map,
            &user_map,
            prepend_proxies,
            append_proxies,
        )? {
            final_map.insert(Value::String("proxies".into()), Value::Sequence(seq));
        }
    }

    // 处理 proxy-groups
    let prepend_proxy_groups = user_map.get(&key_prepend_proxy_groups);
    let append_proxy_groups = user_map.get(&key_append_proxy_groups);
    if prepend_proxy_groups.is_some() || append_proxy_groups.is_some() {
        if let Some(seq) = merge_sequence_field(
            "proxy-groups",
            &remote_map,
            &user_map,
            prepend_proxy_groups,
            append_proxy_groups,
        )? {
            final_map.insert(Value::String("proxy-groups".into()), Value::Sequence(seq));
        }
    }

    Ok(Value::Mapping(final_map))
}

pub(crate) fn merged_config_path(root: &PathBuf) -> PathBuf {
    let mut path = root.clone();
    path.push("config");
    path.push("merged.yaml");
    path
}

fn save_merged_config(root: &PathBuf, value: &serde_yaml::Value) -> Result<(), String> {
    use std::fs;
    use std::io::Write;

    let path = merged_config_path(root);
    let parent = path
        .parent()
        .ok_or_else(|| format!("invalid merged.yaml path: {}", path.display()))?;

    fs::create_dir_all(parent)
        .map_err(|err| format!("failed to create config dir at {}: {err}", parent.display()))?;

    let content = serde_yaml::to_string(value)
        .map_err(|err| format!("failed to serialize merged config: {err}"))?;

    let tmp_path = path.with_extension("yaml.tmp");
    {
        let mut file = fs::File::create(&tmp_path).map_err(|err| {
            format!(
                "failed to create temp merged config at {}: {err}",
                tmp_path.display()
            )
        })?;
        file.write_all(content.as_bytes()).map_err(|err| {
            format!(
                "failed to write temp merged config at {}: {err}",
                tmp_path.display()
            )
        })?;
        file.flush().map_err(|err| {
            format!(
                "failed to flush temp merged config at {}: {err}",
                tmp_path.display()
            )
        })?;
    }

    fs::rename(&tmp_path, &path).map_err(|err| {
        format!(
            "failed to move temp merged config from {} to {}: {err}",
            tmp_path.display(),
            path.display()
        )
    })
}

fn core_defaults_path(root: &PathBuf) -> PathBuf {
    let mut path = root.clone();
    path.push("config");
    path.push("core-defaults.yaml");
    path
}

const CORE_DEFAULTS_YAML: &str = r#" # Core defaults for camofy (router)
mode: rule
mixed-port: 7897
allow-lan: false
log-level: warning
ipv6: true
external-controller-unix: /tmp/verge/clash-verge-service.sock
tun:
  enable: true
  stack: gvisor
  auto-route: true
  strict-route: false
  auto-detect-interface: true
  dns-hijack:
    - any:53
profile:
  store-selected: true
sniffer:
  sniff:
    TLS:
      ports:
      - 1-65535
      override-destination: true
    HTTP:
      ports:
      - 1-65535
      override-destination: true
  enable: true
  skip-domain:
  - Mijia Cloud
  - dlg.io.mi.com
  parse-pure-ip: false
  force-dns-mapping: true
  override-destination: true
dns:
  ipv6: true
  enable: true
  listen: 0.0.0.0:1053
  use-hosts: false
  default-nameserver:
  - 119.29.29.29
  - 223.5.5.5
  - 223.6.6.6
  - 8.8.4.4
  - 8.8.8.8
  nameserver:
  - https://durable0762.com:44443/dns-query
  - https://delirium9599.com:44443/dns-query
  - https://cf-cdn.delirium9599.com:443/dns-query
  fake-ip-range: 198.18.0.1/15
  fake-ip-filter:
  - '*.lan'
  - '*.localdomain'
  - '*.example'
  - '*.invalid'
  - '*.localhost'
  - '*.test'
  - '*.local'
  - '*.home.arpa'
"#;

pub fn generate_merged_config(root: &PathBuf) -> Result<(), String> {
    let config = load_app_config(root)?;

    // 远程订阅侧配置（可选：如果未设置活跃订阅，则视为无远程配置）
    let remote_value = if let Some(active_id) = config.active_subscription_id.as_deref() {
        let Some(profile) = config
            .profiles
            .iter()
            .find(|p| matches!(p.profile_type, ProfileType::Remote) && p.id == active_id)
        else {
            return Err("active_subscription_not_found".to_string());
        };

        let path = profile_file_path(root, profile);
        if !path.is_file() {
            return Err(format!(
                "subscription config file not found at {}",
                path.display()
            ));
        }

        Some(load_yaml_file(&path)?)
    } else {
        None
    };

    // 用户侧配置（可选）
    let user_value = if let Some(active_id) = config.active_user_profile_id.as_deref() {
        let Some(profile) = config
            .profiles
            .iter()
            .find(|p| matches!(p.profile_type, ProfileType::User) && p.id == active_id)
        else {
            return Err("active_user_profile_not_found".to_string());
        };

        let path = profile_file_path(root, profile);
        if !path.is_file() {
            return Err(format!(
                "user profile config file not found at {}",
                path.display()
            ));
        }

        Some(load_yaml_file(&path)?)
    } else {
        None
    };

    let mut merged = merge_yaml_configs(remote_value.as_ref(), user_value.as_ref())
        .map_err(|err| format!("config merge failed: {err}"))?;

    // 将 core-defaults.yaml 作为“基础配置” include 进来，再走一遍通用合并逻辑：
    // defaults 作为 remote，profiles merge 结果作为 user，保证用户/订阅可覆盖默认值。
    let defaults_path = core_defaults_path(root);
    let defaults_value_opt = {
        use std::fs;
        use std::io::Write;

        if !defaults_path.is_file() {
            if let Some(parent) = defaults_path.parent() {
                if let Err(err) = fs::create_dir_all(parent) {
                    tracing::error!(
                        "failed to create core-defaults dir {}: {err}",
                        parent.display()
                    );
                }
            }
            if let Err(err) = fs::File::create(&defaults_path)
                .and_then(|mut f| f.write_all(CORE_DEFAULTS_YAML.as_bytes()))
            {
                tracing::error!(
                    "failed to write default core-defaults.yaml at {}: {err}",
                    defaults_path.display()
                );
                None
            } else {
                match load_yaml_file(&defaults_path) {
                    Ok(v) => Some(v),
                    Err(err) => {
                        tracing::error!("failed to parse default core-defaults.yaml: {err}");
                        None
                    }
                }
            }
        } else {
            match load_yaml_file(&defaults_path) {
                Ok(v) => Some(v),
                Err(err) => {
                    tracing::error!("failed to load core-defaults.yaml: {err}");
                    None
                }
            }
        }
    };

    if let Some(defaults_value) = defaults_value_opt.as_ref() {
        merged = merge_yaml_configs(Some(&merged), Some(defaults_value))
            .map_err(|err| format!("config merge failed: {err}"))?;
    }

    save_merged_config(root, &merged)
}

pub async fn get_merged_config() -> Json<ApiResponse<serde_json::Value>> {
    use std::fs;

    let state = app_state();
    let path = merged_config_path(&state.data_root);

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                return Json(ApiResponse {
                    code: "merged_config_not_found".to_string(),
                    message: "merged.yaml not found".to_string(),
                    data: None,
                });
            }
            let msg = format!("failed to read merged.yaml at {}: {err}", path.display());
            tracing::error!("{msg}");
            return Json(ApiResponse {
                code: "merged_config_read_failed".to_string(),
                message: msg,
                data: None,
            });
        }
    };

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "success".to_string(),
        data: Some(serde_json::json!({ "content": content })),
    })
}

#[cfg(test)]
mod tests {
    use super::{generate_merged_config, merged_config_path};
    use crate::{AppConfig, ProfileMeta, ProfileType, save_app_config};
    use std::fs;
    use std::path::PathBuf;

    fn temp_root(suffix: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dir.push(format!("camofy-test-{suffix}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn core_defaults_applied_when_no_profiles() {
        let root = temp_root("core-defaults");

        generate_merged_config(&root).expect("generate_merged_config failed");

        let path = merged_config_path(&root);
        let content = fs::read_to_string(&path).expect("read merged.yaml");
        let value: serde_yaml::Value = serde_yaml::from_str(&content).expect("parse merged.yaml");

        assert_eq!(
            value
                .get("external-controller-unix")
                .and_then(|v| v.as_str()),
            Some("/tmp/verge/clash-verge-service.sock")
        );
        assert_eq!(value.get("mixed-port").and_then(|v| v.as_i64()), Some(7897));
        assert_eq!(value.get("mode").and_then(|v| v.as_str()), Some("rule"));
        assert_eq!(
            value
                .get("profile")
                .and_then(|p| p.get("store-selected"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn user_profile_overrides_core_defaults() {
        let root = temp_root("core-overrides");

        let profile_id = "user1".to_string();
        let profile = ProfileMeta {
            id: profile_id.clone(),
            name: "test".to_string(),
            profile_type: ProfileType::User,
            path: "user-profiles/user1.yaml".to_string(),
            url: None,
            last_fetch_time: None,
            last_fetch_status: None,
            last_modified_time: None,
        };

        let mut app_cfg = AppConfig::default();
        app_cfg.profiles.push(profile);
        app_cfg.active_user_profile_id = Some(profile_id);
        save_app_config(&root, &app_cfg).expect("save_app_config failed");

        let mut profile_dir = root.clone();
        profile_dir.push("config");
        profile_dir.push("user-profiles");
        fs::create_dir_all(&profile_dir).unwrap();

        let mut profile_path = profile_dir;
        profile_path.push("user1.yaml");
        fs::write(&profile_path, "mixed-port: 8888\nmode: global\n").expect("write user profile");

        generate_merged_config(&root).expect("generate_merged_config failed");

        let merged_path = merged_config_path(&root);
        let merged_content =
            fs::read_to_string(&merged_path).expect("read merged.yaml after override");
        let value: serde_yaml::Value =
            serde_yaml::from_str(&merged_content).expect("parse merged.yaml after override");

        assert_eq!(value.get("mixed-port").and_then(|v| v.as_i64()), Some(8888));
        assert_eq!(value.get("mode").and_then(|v| v.as_str()), Some("global"));
    }
}
