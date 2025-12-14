use std::collections::HashMap;
use std::path::PathBuf;

use axum::{extract::Path, Json};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::app::app_state;
use crate::core::ensure_controller_secret;
use crate::{ApiResponse, ProxySelectionRecord};

const MIHOMO_SOCKET_PATH: &str = "/tmp/verge/clash-verge-service.sock";
const DEFAULT_TEST_URL: &str = "https://www.gstatic.com/generate_204";
const DEFAULT_TEST_TIMEOUT_MS: u32 = 5000;

#[derive(Serialize)]
pub struct ProxyNodeDto {
    pub name: String,
    #[serde(rename = "type")]
    pub proxy_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay: Option<u32>,
}

#[derive(Serialize)]
pub struct ProxyGroupDto {
    pub name: String,
    #[serde(rename = "type")]
    pub group_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    pub nodes: Vec<ProxyNodeDto>,
}

#[derive(Serialize)]
pub struct ProxiesViewDto {
    pub groups: Vec<ProxyGroupDto>,
}

#[derive(Serialize)]
pub struct GroupDelayResultDto {
    pub node: String,
    pub delay_ms: u32,
}

#[derive(Serialize)]
pub struct GroupDelayResponseDto {
    pub group: String,
    pub url: String,
    pub timeout_ms: u32,
    pub results: Vec<GroupDelayResultDto>,
}

#[derive(Serialize)]
pub struct ProxyDelayResponseDto {
    pub group: String,
    pub node: String,
    pub url: String,
    pub timeout_ms: u32,
    pub delay_ms: u32,
}

#[derive(Deserialize)]
struct ProxiesRaw {
    proxies: HashMap<String, ProxyRaw>,
}

#[derive(Deserialize)]
struct ProxyRaw {
    name: String,
    #[serde(rename = "type")]
    proxy_type: Option<String>,
    #[serde(default)]
    all: Option<Vec<String>>,
    #[serde(default)]
    now: Option<String>,
    #[serde(default)]
    history: Vec<DelayEntry>,
}

#[derive(Deserialize)]
struct DelayEntry {
    #[serde(default)]
    delay: Option<u32>,
}

#[derive(Deserialize)]
struct ErrorResponseBody {
    #[serde(default)]
    message: Option<String>,
}

fn build_http_request(
    method: &str,
    path: &str,
    body: Option<&str>,
    secret: &str,
) -> String {
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };

    let mut req = String::new();
    req.push_str(&format!("{method} {path} HTTP/1.1\r\n"));
    req.push_str("Host: 127.0.0.1\r\n");
    req.push_str("Accept: application/json\r\n");
    req.push_str("Connection: close\r\n");
    req.push_str(&format!("Authorization: Bearer {secret}\r\n"));

    if let Some(body_str) = body {
        req.push_str("Content-Type: application/json\r\n");
        req.push_str(&format!("Content-Length: {}\r\n", body_str.len()));
        req.push_str("\r\n");
        req.push_str(body_str);
    } else {
        req.push_str("\r\n");
    }

    req
}

async fn read_header(reader: &mut BufReader<&mut UnixStream>) -> Result<String, String> {
    let mut header = String::new();
    loop {
        let mut line = String::new();
        let size = reader
            .read_line(&mut line)
            .await
            .map_err(|err| format!("failed to read response header: {err}"))?;
        if size == 0 {
            return Err("no response from mihomo".to_string());
        }
        header.push_str(&line);
        if line == "\r\n" {
            break;
        }
    }
    Ok(header)
}

async fn read_chunked_body(reader: &mut BufReader<&mut UnixStream>) -> Result<String, String> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        reader
            .read_line(&mut size_line)
            .await
            .map_err(|err| format!("failed to read chunk size: {err}"))?;
        let size_line = size_line.trim();
        if size_line.is_empty() {
            continue;
        }
        let chunk_size = usize::from_str_radix(size_line, 16)
            .map_err(|err| format!("failed to parse chunk size: {err}"))?;

        if chunk_size == 0 {
            let mut _end = String::new();
            reader
                .read_line(&mut _end)
                .await
                .map_err(|err| format!("failed to read chunk terminator: {err}"))?;
            break;
        }

        let mut chunk_data = vec![0u8; chunk_size];
        reader
            .read_exact(&mut chunk_data)
            .await
            .map_err(|err| format!("failed to read chunk data: {err}"))?;
        body.extend_from_slice(&chunk_data);

        let mut _crlf = String::new();
        reader
            .read_line(&mut _crlf)
            .await
            .map_err(|err| format!("failed to read chunk CRLF: {err}"))?;
    }

    String::from_utf8(body).map_err(|err| format!("failed to decode chunked body as utf-8: {err}"))
}

async fn send_mihomo_request(
    method: &str,
    path: &str,
    body: Option<&str>,
    secret: &str,
) -> Result<(u16, String), String> {
    let mut stream = UnixStream::connect(MIHOMO_SOCKET_PATH)
        .await
        .map_err(|err| format!("failed to connect to mihomo unix socket at {MIHOMO_SOCKET_PATH}: {err}"))?;

    let request = build_http_request(method, path, body, secret);
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|err| format!("failed to write request to mihomo: {err}"))?;
    stream
        .flush()
        .await
        .map_err(|err| format!("failed to flush request to mihomo: {err}"))?;

    let mut reader = BufReader::new(&mut stream);

    let header = read_header(&mut reader).await?;

    let mut content_length: Option<usize> = None;
    let mut is_chunked = false;
    for line in header.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length: ") {
            if let Ok(len) = v.trim().parse::<usize>() {
                content_length = Some(len);
            }
        }
        if lower.contains("transfer-encoding: chunked") {
            is_chunked = true;
        }
    }

    let body_str = if is_chunked {
        read_chunked_body(&mut reader).await?
    } else if let Some(len) = content_length {
        let mut buf = vec![0u8; len];
        reader
            .read_exact(&mut buf)
            .await
            .map_err(|err| format!("failed to read response body: {err}"))?;
        String::from_utf8(buf)
            .map_err(|err| format!("failed to decode response body as utf-8: {err}"))?
    } else {
        let mut buf = String::new();
        reader
            .read_to_string(&mut buf)
            .await
            .map_err(|err| format!("failed to read response body: {err}"))?;
        buf
    };

    let mut lines = header.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| "invalid mihomo response: missing status line".to_string())?;
    let mut parts = status_line.split_whitespace();
    let _ = parts
        .next()
        .ok_or_else(|| "invalid mihomo response: missing http version".to_string())?;
    let code_str = parts
        .next()
        .ok_or_else(|| "invalid mihomo response: missing status code".to_string())?;
    let status_code: u16 = code_str
        .parse()
        .map_err(|err| format!("invalid mihomo status code: {err}"))?;

    Ok((status_code, body_str))
}

fn map_error_from_body(status: u16, body: &str) -> String {
    if body.is_empty() {
        return format!("mihomo returned {} with empty body", status);
    }
    match serde_json::from_str::<ErrorResponseBody>(body) {
        Ok(err_body) => err_body
            .message
            .unwrap_or_else(|| format!("mihomo returned status {status}")),
        Err(_) => format!("mihomo returned status {status}: {body}"),
    }
}

async fn delay_group(
    secret: &str,
    group: &str,
    url: &str,
    timeout_ms: u32,
) -> Result<HashMap<String, u32>, String> {
    let group_enc = encode_path_segment(group);
    let url_enc = encode_path_segment(url);
    let path = format!(
        "/group/{group_enc}/delay?url={}&timeout={}",
        url_enc, timeout_ms
    );

    let (status, body) = send_mihomo_request("GET", &path, None, secret).await?;
    if (200..300).contains(&status) {
        serde_json::from_str::<HashMap<String, u32>>(&body).map_err(|err| {
            format!("failed to parse group delay response for {group}: {err}")
        })
    } else {
        Err(map_error_from_body(status, &body))
    }
}

async fn delay_proxy(
    secret: &str,
    proxy: &str,
    url: &str,
    timeout_ms: u32,
) -> Result<u32, String> {
    let proxy_enc = encode_path_segment(proxy);
    let url_enc = encode_path_segment(url);
    let path = format!(
        "/proxies/{proxy_enc}/delay?url={}&timeout={}",
        url_enc, timeout_ms
    );

    let (status, body) = send_mihomo_request("GET", &path, None, secret).await?;
    if (200..300).contains(&status) {
        #[derive(Deserialize)]
        struct DelayBody {
            delay: u32,
        }
        serde_json::from_str::<DelayBody>(&body)
            .map(|v| v.delay)
            .map_err(|err| {
                format!("failed to parse proxy delay response for {proxy}: {err}")
            })
    } else {
        // 超时时 mihomo 可能返回错误体，这里统一映射为 delay=0。
        tracing::debug!(
            "proxy delay for '{}' returned non-success status {}; treating as timeout",
            proxy,
            status
        );
        Ok(0)
    }
}

/// 使用当前的 `merged.yaml` 向 Mihomo 发送“重新加载配置”请求。
///
/// 约定：调用方应保证 `merged.yaml` 已经根据最新配置生成。
pub async fn reload_config_with_merged(root: &PathBuf) -> Result<(), String> {
    let secret = ensure_controller_secret(root)
        .map_err(|err| format!("failed to ensure controller secret: {err}"))?;

    let path = crate::user_profiles::merged_config_path(root);
    if !path.is_file() {
        return Err(format!(
            "merged config file not found at {}",
            path.display()
        ));
    }

    let path_str = path
        .to_str()
        .ok_or_else(|| format!("merged config path is not valid utf-8: {}", path.display()))?;

    let body = serde_json::json!({
        "path": path_str,
        "force": true,
    });
    let body_str = serde_json::to_string(&body)
        .map_err(|err| format!("failed to serialize reload-config body: {err}"))?;

    let (status, resp_body) =
        send_mihomo_request("PUT", "/configs", Some(&body_str), &secret).await?;

    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(map_error_from_body(status, &resp_body))
    }
}

async fn fetch_proxies_view(secret: &str) -> Result<ProxiesViewDto, String> {
    let (status, body) = send_mihomo_request("GET", "/proxies", None, secret).await?;

    if status < 200 || status >= 300 {
        let msg = map_error_from_body(status, &body);
        return Err(msg);
    }

    let raw: ProxiesRaw = serde_json::from_str(&body)
        .map_err(|err| format!("failed to parse mihomo /proxies response: {err}"))?;

    // 首先将所有“具备 all 字段的代理组（排除 GLOBAL）”收集到一个临时映射中，
    // 键为组名，值为组的详细信息。这样后续可以根据 GLOBAL.all 的顺序来重建
    // 代理组列表，最大程度接近 Mihomo / Clash Verge 的展示顺序。
    let mut groups_map: std::collections::HashMap<String, ProxyGroupDto> =
        std::collections::HashMap::new();

    for proxy in raw.proxies.values() {
        if proxy.name == "GLOBAL" {
            continue;
        }
        let Some(all_nodes) = proxy.all.as_ref() else {
            continue;
        };

        let mut nodes = Vec::new();
        for node_name in all_nodes {
            let node_entry = raw
                .proxies
                .get(node_name)
                .or_else(|| raw.proxies.get(node_name.as_str()));

            if let Some(node) = node_entry {
                let delay = node.history.last().and_then(|h| h.delay);
                nodes.push(ProxyNodeDto {
                    name: node.name.clone(),
                    proxy_type: node
                        .proxy_type
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                    delay,
                });
            } else {
                nodes.push(ProxyNodeDto {
                    name: node_name.clone(),
                    proxy_type: "unknown".to_string(),
                    delay: None,
                });
            }
        }

        groups_map.insert(
            proxy.name.clone(),
            ProxyGroupDto {
                name: proxy.name.clone(),
                group_type: proxy
                    .proxy_type
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                now: proxy.now.clone(),
                nodes,
            },
        );
    }

    let mut groups: Vec<ProxyGroupDto> = Vec::new();

    // 若存在 GLOBAL 组且其 all 字段中定义了其他组的顺序，则优先按该顺序
    // 将对应的组提取出来，以接近 Clash Verge 中基于 GLOBAL.all 重排的效果。
    if let Some(global) = raw.proxies.get("GLOBAL") {
        if let Some(global_all) = global.all.as_ref() {
            for group_name in global_all {
                if let Some(group) = groups_map.remove(group_name) {
                    groups.push(group);
                }
            }
        }
    }

    // 其余未在 GLOBAL.all 中出现的组，按 HashMap 内部顺序追加。
    // 这与 Clash Verge 中“剩余组保持原始顺序”的行为大致一致。
    groups.extend(groups_map.into_values());

    Ok(ProxiesViewDto { groups })
}

fn encode_path_segment(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

async fn select_node_for_group(secret: &str, group: &str, node: &str) -> Result<(), String> {
    let path = format!("/proxies/{}", encode_path_segment(group));
    let body = serde_json::json!({ "name": node });
    let body_str =
        serde_json::to_string(&body).map_err(|err| format!("failed to serialize select-node request body: {err}"))?;

    let (status, resp_body) = send_mihomo_request("PUT", &path, Some(&body_str), secret).await?;
    if status < 200 || status >= 300 {
        let msg = map_error_from_body(status, &resp_body);
        return Err(msg);
    }

    Ok(())
}

/// 在 Mihomo 核心运行且配置已加载的前提下，根据当前活跃配置组合下保存的
/// 代理选择快照，尝试恢复各个代理组的“已选节点”。
///
/// - 若当前没有任何保存的选择，则直接返回 Ok(())。
/// - 若部分组或节点已不存在，则跳过并记录日志，不影响其他组的恢复。
pub(crate) async fn apply_saved_proxy_selection() -> Result<(), String> {
    let state = app_state();

    let selections: Vec<ProxySelectionRecord> =
        match crate::get_proxy_selections_for_active_profile() {
            Some(list) if !list.is_empty() => list,
            _ => {
                tracing::debug!(
                    "no saved proxy selections for current profile; skip apply"
                );
                return Ok(());
            }
        };

    let secret = ensure_controller_secret(&state.data_root)
        .map_err(|err| format!("failed to ensure controller secret: {err}"))?;

    let view = match fetch_proxies_view(&secret).await {
        Ok(v) => v,
        Err(err) => {
            return Err(format!(
                "failed to fetch proxies view when applying saved selections: {err}"
            ));
        }
    };

    // 仅对常见可选代理组类型尝试恢复，避免对不可选组误操作。
    const SELECTABLE_TYPES: [&str; 4] =
        ["Selector", "URLTest", "Fallback", "LoadBalance"];

    let mut errors: Vec<String> = Vec::new();
    let mut applied_count: usize = 0;

    for record in selections {
        let Some(group) = view
            .groups
            .iter()
            .find(|g| g.name == record.group)
        else {
            tracing::debug!(
                "saved proxy selection group '{}' not found in current view; skip",
                record.group
            );
            continue;
        };

        if !SELECTABLE_TYPES
            .iter()
            .any(|t| t.eq_ignore_ascii_case(group.group_type.as_str()))
            && group.name != "GLOBAL"
        {
            tracing::debug!(
                "group '{}' (type '{}') is not selectable; skip saved selection",
                group.name,
                group.group_type
            );
            continue;
        }

        let target = record.node;

        // 已经是目标节点则无需重复切换。
        if group.now.as_deref() == Some(target.as_str()) {
            continue;
        }

        let exists = group
            .nodes
            .iter()
            .any(|n| n.name == target);
        if !exists {
            tracing::debug!(
                "saved proxy '{}' not found in group '{}'; skip",
                target,
                group.name
            );
            continue;
        }

        if let Err(err) =
            select_node_for_group(&secret, &group.name, target.as_str()).await
        {
            let msg = format!(
                "failed to apply saved selection for group '{}' -> '{}': {err}",
                group.name, target
            );
            tracing::warn!("{msg}");
            errors.push(msg);
        } else {
            applied_count += 1;
            tracing::info!(
                "applied saved proxy selection: group='{}', node='{}'",
                group.name,
                target
            );
        }
    }

    if applied_count > 0 {
        tracing::info!(
            "applied {} saved proxy selections for current profile",
            applied_count
        );
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "some saved proxy selections failed to apply: {}",
            errors.join("; ")
        ))
    }
}

pub async fn get_proxies() -> Json<ApiResponse<ProxiesViewDto>> {
    let state = app_state();

    let secret = match ensure_controller_secret(&state.data_root) {
        Ok(s) => s,
        Err(err) => {
            tracing::error!("{err}");
            return Json(ApiResponse {
                code: "mihomo_secret_error".to_string(),
                message: err,
                data: None,
            });
        }
    };

    match fetch_proxies_view(&secret).await {
        Ok(view) => Json(ApiResponse {
            code: "ok".to_string(),
            message: "success".to_string(),
            data: Some(view),
        }),
        Err(err) => {
            tracing::error!("failed to fetch mihomo proxies: {err}");
            Json(ApiResponse {
                code: "mihomo_proxies_failed".to_string(),
                message: err,
                data: None,
            })
        }
    }
}

#[derive(Deserialize)]
pub struct SelectProxyRequest {
    pub name: String,
}

pub async fn select_proxy(
    Path(group): Path<String>,
    Json(body): Json<SelectProxyRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let state = app_state();

    let secret = match ensure_controller_secret(&state.data_root) {
        Ok(s) => s,
        Err(err) => {
            tracing::error!("{err}");
            return Json(ApiResponse {
                code: "mihomo_secret_error".to_string(),
                message: err,
                data: None,
            });
        }
    };

    if body.name.trim().is_empty() {
        return Json(ApiResponse {
            code: "mihomo_invalid_proxy_name".to_string(),
            message: "proxy name cannot be empty".to_string(),
            data: None,
        });
    }

    match select_node_for_group(&secret, &group, &body.name).await {
        Ok(()) => {
            // 选择节点成功后，记录当前配置组合下的代理选择快照，便于内核重启后恢复。
            if let Err(err) =
                crate::update_proxy_selection_for_current_profile(&group, &body.name)
            {
                tracing::error!(
                    "failed to persist proxy selection for group {group}: {err}"
                );
            }

            Json(ApiResponse {
                code: "ok".to_string(),
                message: "selected".to_string(),
                data: Some(serde_json::json!({})),
            })
        }
        Err(err) => {
            tracing::error!("failed to select proxy for group {group}: {err}");
            Json(ApiResponse {
                code: "mihomo_select_failed".to_string(),
                message: err,
                data: None,
            })
        }
    }
}

#[derive(Deserialize)]
pub struct GroupDelayRequest {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u32>,
    /// "group" 使用 mihomo group delay 接口；"nodes" 针对节点逐个测试。
    #[serde(default)]
    pub mode: Option<String>,
    /// 当 mode 为 "nodes" 时，如指定则仅测试该列表中的节点。
    #[serde(default)]
    pub nodes: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct NodeDelayRequest {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u32>,
}

pub async fn test_group_delay(
    Path(group): Path<String>,
    Json(body): Json<GroupDelayRequest>,
) -> Json<ApiResponse<GroupDelayResponseDto>> {
    let state = app_state();

    let (running, _) = crate::core::core_running_status(&state.data_root);
    if !running {
        return Json(ApiResponse {
            code: "core_not_running".to_string(),
            message: "core is not running".to_string(),
            data: None,
        });
    }

    let secret = match ensure_controller_secret(&state.data_root) {
        Ok(s) => s,
        Err(err) => {
            tracing::error!("{err}");
            return Json(ApiResponse {
                code: "mihomo_secret_error".to_string(),
                message: err,
                data: None,
            });
        }
    };

    let url = body
        .url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_TEST_URL)
        .to_string();
    let timeout_ms = body.timeout_ms.unwrap_or(DEFAULT_TEST_TIMEOUT_MS);

    let mut results: Vec<GroupDelayResultDto> = Vec::new();

    // 逐个节点测试：先获取当前组所有节点，再按需要筛选。
    let view = match fetch_proxies_view(&secret).await {
        Ok(v) => v,
        Err(err) => {
            let msg = format!(
                "failed to fetch proxies view before node-based delay test: {err}"
            );
            tracing::error!("{msg}");
            return Json(ApiResponse {
                code: "mihomo_proxies_failed".to_string(),
                message: msg,
                data: None,
            });
        }
    };

    let Some(group_view) = view.groups.iter().find(|g| g.name == group) else {
        let msg = format!("proxy group '{}' not found", group);
        return Json(ApiResponse {
            code: "mihomo_group_not_found".to_string(),
            message: msg,
            data: None,
        });
    };

    let filter_nodes: Option<std::collections::HashSet<String>> =
        body.nodes.map(|ns| ns.into_iter().collect());

    for node in &group_view.nodes {
        if let Some(ref filter) = filter_nodes {
            if !filter.contains(&node.name) {
                continue;
            }
        }

        match delay_proxy(&secret, &node.name, &url, timeout_ms).await {
            Ok(delay_ms) => {
                results.push(GroupDelayResultDto {
                    node: node.name.clone(),
                    delay_ms,
                });
            }
            Err(err) => {
                tracing::warn!(
                    "delay test for proxy '{}' in group '{}' failed: {err}",
                    node.name,
                    group
                );
                results.push(GroupDelayResultDto {
                    node: node.name.clone(),
                    delay_ms: 0,
                });
            }
        }
    }

    tracing::info!(
        "delay test for group '{}' finished: {} nodes, url={}, timeout_ms={}",
        group,
        results.len(),
        url,
        timeout_ms
    );

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "success".to_string(),
        data: Some(GroupDelayResponseDto {
            group,
            url,
            timeout_ms,
            results,
        }),
    })
}

pub async fn test_node_delay(
    Path((group, node)): Path<(String, String)>,
    Json(body): Json<NodeDelayRequest>,
) -> Json<ApiResponse<ProxyDelayResponseDto>> {
    let state = app_state();

    let (running, _) = crate::core::core_running_status(&state.data_root);
    if !running {
        return Json(ApiResponse {
            code: "core_not_running".to_string(),
            message: "core is not running".to_string(),
            data: None,
        });
    }

    let secret = match ensure_controller_secret(&state.data_root) {
        Ok(s) => s,
        Err(err) => {
            tracing::error!("{err}");
            return Json(ApiResponse {
                code: "mihomo_secret_error".to_string(),
                message: err,
                data: None,
            });
        }
    };

    let url = body
        .url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_TEST_URL)
        .to_string();
    let timeout_ms = body.timeout_ms.unwrap_or(DEFAULT_TEST_TIMEOUT_MS);

    let delay_ms = match delay_proxy(&secret, &node, &url, timeout_ms).await {
        Ok(d) => d,
        Err(err) => {
            let msg = format!(
                "delay test for proxy '{}' in group '{}' failed: {err}",
                node, group
            );
            tracing::error!("{msg}");
            return Json(ApiResponse {
                code: "mihomo_delay_proxy_failed".to_string(),
                message: msg,
                data: None,
            });
        }
    };

    tracing::info!(
        "delay test for proxy '{}' in group '{}' finished: delay={}ms, url={}, timeout_ms={}",
        node,
        group,
        delay_ms,
        url,
        timeout_ms
    );

    Json(ApiResponse {
        code: "ok".to_string(),
        message: "success".to_string(),
        data: Some(ProxyDelayResponseDto {
            group,
            node,
            url,
            timeout_ms,
            delay_ms,
        }),
    })
}
