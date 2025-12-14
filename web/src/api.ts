import type {
  ApiResponse,
  AuthLoginResponse,
  CoreInfo,
  CoreStatus,
  LogResponse,
  MergedConfig,
  ProxiesView,
  Settings,
  Subscription,
  SubscriptionListResponse,
  UserProfileDetail,
  UserProfileListResponse,
  UserProfileSummary,
  ScheduledTaskConfig,
  GroupDelayResponse,
} from './types'

export type AuthedFetch = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>

async function requestJson<T>(
  authedFetch: AuthedFetch,
  input: RequestInfo | URL,
  init?: RequestInit,
): Promise<ApiResponse<T>> {
  const res = await authedFetch(input, init)
  return (await res.json()) as ApiResponse<T>
}

export async function fetchSettings(
  authedFetch: AuthedFetch,
): Promise<ApiResponse<Settings>> {
  return requestJson<Settings>(authedFetch, '/api/settings')
}

export async function login(
  password: string,
): Promise<ApiResponse<AuthLoginResponse>> {
  const res = await fetch('/api/auth/login', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ password }),
  })
  return (await res.json()) as ApiResponse<AuthLoginResponse>
}

export async function listSubscriptions(
  authedFetch: AuthedFetch,
): Promise<Subscription[]> {
  const body = await requestJson<SubscriptionListResponse>(
    authedFetch,
    '/api/subscriptions',
  )
  if (body.code !== 'ok' || !body.data) {
    throw new Error(body.message || '加载订阅列表失败')
  }
  return body.data.subscriptions
}

export async function saveSubscription(
  authedFetch: AuthedFetch,
  params: { id?: string | null; name: string; url: string },
): Promise<Subscription> {
  const payload = { name: params.name.trim(), url: params.url.trim() }
  const path = params.id
    ? `/api/subscriptions/${encodeURIComponent(params.id)}`
    : '/api/subscriptions'
  const method = params.id ? 'PUT' : 'POST'

  const body = await requestJson<Subscription>(authedFetch, path, {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  })

  if (body.code !== 'ok' || !body.data) {
    throw new Error(body.message || '保存订阅失败')
  }
  return body.data
}

export async function deleteSubscription(
  authedFetch: AuthedFetch,
  id: string,
): Promise<void> {
  const body = await requestJson<unknown>(
    authedFetch,
    `/api/subscriptions/${encodeURIComponent(id)}`,
    { method: 'DELETE' },
  )
  if (body.code !== 'ok') {
    throw new Error(body.message || '删除订阅失败')
  }
}

export async function activateSubscription(
  authedFetch: AuthedFetch,
  id: string,
): Promise<void> {
  const body = await requestJson<unknown>(
    authedFetch,
    `/api/subscriptions/${encodeURIComponent(id)}/activate`,
    { method: 'POST' },
  )
  if (body.code !== 'ok') {
    throw new Error(body.message || '设置活跃订阅失败')
  }
}

export async function fetchSubscriptionContent(
  authedFetch: AuthedFetch,
  id: string,
): Promise<void> {
  const body = await requestJson<unknown>(
    authedFetch,
    `/api/subscriptions/${encodeURIComponent(id)}/fetch`,
    { method: 'POST' },
  )
  if (body.code !== 'ok') {
    throw new Error(body.message || '拉取订阅失败')
  }
}

export async function getCoreInfo(
  authedFetch: AuthedFetch,
): Promise<CoreInfo | null> {
  const body = await requestJson<CoreInfo>(authedFetch, '/api/core')
  if (body.code === 'ok' && body.data) {
    return body.data
  }
  if (body.message) {
    throw new Error(body.message)
  }
  return null
}

export async function getCoreStatus(
  authedFetch: AuthedFetch,
): Promise<CoreStatus | null> {
  const body = await requestJson<CoreStatus>(authedFetch, '/api/core/status')
  if (body.code === 'ok' && body.data) {
    return body.data
  }
  if (body.message) {
    throw new Error(body.message)
  }
  return null
}

export async function downloadCore(
  authedFetch: AuthedFetch,
): Promise<CoreInfo> {
  const body = await requestJson<CoreInfo>(authedFetch, '/api/core/download', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  })
  if (body.code !== 'ok' || !body.data) {
    throw new Error(body.message || '内核下载失败')
  }
  return body.data
}

export async function startCore(
  authedFetch: AuthedFetch,
): Promise<void> {
  const body = await requestJson<{ operation?: string }>(authedFetch, '/api/core/start', {
    method: 'POST',
  })
  if (body.code !== 'ok') {
    throw new Error(body.message || '启动内核失败')
  }
}

export async function stopCore(authedFetch: AuthedFetch): Promise<void> {
  const body = await requestJson<{ operation?: string }>(
    authedFetch,
    '/api/core/stop',
    {
      method: 'POST',
    },
  )
  if (body.code !== 'ok') {
    throw new Error(body.message || '停止内核失败')
  }
}

export async function restartCore(
  authedFetch: AuthedFetch,
): Promise<void> {
  const body = await requestJson<{ operation?: string }>(
    authedFetch,
    '/api/core/restart',
    {
      method: 'POST',
    },
  )
  if (body.code !== 'ok') {
    throw new Error(body.message || '重启内核失败')
  }
}

export async function listUserProfiles(
  authedFetch: AuthedFetch,
): Promise<UserProfileSummary[]> {
  const body = await requestJson<UserProfileListResponse>(
    authedFetch,
    '/api/user-profiles',
  )
  if (body.code !== 'ok' || !body.data) {
    throw new Error(body.message || '加载用户配置列表失败')
  }
  return body.data.user_profiles
}

export async function getUserProfileDetail(
  authedFetch: AuthedFetch,
  id: string,
): Promise<UserProfileDetail> {
  const body = await requestJson<UserProfileDetail>(
    authedFetch,
    `/api/user-profiles/${encodeURIComponent(id)}`,
  )
  if (body.code !== 'ok' || !body.data) {
    throw new Error(body.message || '加载用户配置失败')
  }
  return body.data
}

export async function createUserProfile(
  authedFetch: AuthedFetch,
  params: { name: string; content: string },
): Promise<UserProfileSummary> {
  const body = await requestJson<UserProfileSummary>(
    authedFetch,
    '/api/user-profiles',
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        name: params.name.trim(),
        content: params.content,
      }),
    },
  )
  if (body.code !== 'ok' || !body.data) {
    throw new Error(body.message || '创建用户配置失败')
  }
  return body.data
}

export async function updateUserProfile(
  authedFetch: AuthedFetch,
  params: { id: string; name: string; content: string },
): Promise<UserProfileDetail> {
  const body = await requestJson<UserProfileDetail>(
    authedFetch,
    `/api/user-profiles/${encodeURIComponent(params.id)}`,
    {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        name: params.name.trim(),
        content: params.content,
      }),
    },
  )
  if (body.code !== 'ok' || !body.data) {
    throw new Error(body.message || '保存用户配置失败')
  }
  return body.data
}

export async function deleteUserProfile(
  authedFetch: AuthedFetch,
  id: string,
): Promise<void> {
  const body = await requestJson<unknown>(
    authedFetch,
    `/api/user-profiles/${encodeURIComponent(id)}`,
    {
      method: 'DELETE',
    },
  )
  if (body.code !== 'ok') {
    throw new Error(body.message || '删除用户配置失败')
  }
}

export async function activateUserProfile(
  authedFetch: AuthedFetch,
  id: string,
): Promise<void> {
  const body = await requestJson<unknown>(
    authedFetch,
    `/api/user-profiles/${encodeURIComponent(id)}/activate`,
    {
      method: 'POST',
    },
  )
  if (body.code !== 'ok') {
    throw new Error(body.message || '设置活跃用户配置失败')
  }
}

export async function getMergedConfig(
  authedFetch: AuthedFetch,
): Promise<string | null> {
  const body = await requestJson<MergedConfig>(
    authedFetch,
    '/api/config/merged',
  )
  if (body.code === 'ok' && body.data) {
    return body.data.content
  }
  if (body.code === 'merged_config_not_found') {
    return null
  }
  throw new Error(body.message || '加载合并配置失败')
}

export async function getLogs(
  authedFetch: AuthedFetch,
): Promise<{ app: string[]; mihomo: string[] }> {
  const [appBody, mihomoBody] = await Promise.all([
    requestJson<LogResponse>(authedFetch, '/api/logs/app'),
    requestJson<LogResponse>(authedFetch, '/api/logs/mihomo'),
  ])

  let app: string[] = []
  let mihomo: string[] = []

  if (appBody.code === 'ok' && appBody.data) {
    app = appBody.data.lines
  } else if (appBody.code !== 'log_not_found' && appBody.message) {
    throw new Error(appBody.message)
  }

  if (mihomoBody.code === 'ok' && mihomoBody.data) {
    mihomo = mihomoBody.data.lines
  } else if (mihomoBody.code !== 'log_not_found' && mihomoBody.message) {
    throw new Error(mihomoBody.message)
  }

  return { app, mihomo }
}

export async function getProxies(
  authedFetch: AuthedFetch,
): Promise<ProxiesView | null> {
  const body = await requestJson<ProxiesView>(
    authedFetch,
    '/api/mihomo/proxies',
  )
  if (body.code === 'ok' && body.data) {
    return body.data
  }
  if (body.code === 'mihomo_proxies_failed') {
    if (body.message) {
      throw new Error(body.message)
    }
    return null
  }
  if (body.message) {
    throw new Error(body.message)
  }
  return null
}

export async function selectProxyNode(
  authedFetch: AuthedFetch,
  groupName: string,
  nodeName: string,
): Promise<void> {
  const body = await requestJson<unknown>(
    authedFetch,
    `/api/mihomo/proxies/${encodeURIComponent(groupName)}/select`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: nodeName }),
    },
  )
  if (body.code !== 'ok') {
    throw new Error(body.message || '切换节点失败')
  }
}

export async function testProxyGroup(
  authedFetch: AuthedFetch,
  groupName: string,
): Promise<GroupDelayResponse> {
  const body = await requestJson<GroupDelayResponse>(
    authedFetch,
    `/api/mihomo/proxies/${encodeURIComponent(groupName)}/test`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({}),
    },
  )
  if (body.code !== 'ok' || !body.data) {
    throw new Error(body.message || '测试节点延迟失败')
  }
  return body.data
}

export async function updateSchedulerSettings(
  authedFetch: AuthedFetch,
  params: {
    subscriptionTask: ScheduledTaskConfig | null
    geoipTask: ScheduledTaskConfig | null
  },
): Promise<Settings> {
  const body = await requestJson<Settings>(authedFetch, '/api/settings', {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      subscription_auto_update: params.subscriptionTask,
      geoip_auto_update: params.geoipTask,
    }),
  })
  if (body.code !== 'ok' || !body.data) {
    throw new Error(body.message || '保存计划任务设置失败')
  }
  return body.data
}
