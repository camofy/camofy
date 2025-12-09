export type ApiResponse<T> = {
  code: string
  message: string
  data?: T
}

export type Subscription = {
  id: string
  name: string
  url: string
  is_active: boolean
  last_fetch_time?: string | null
  last_fetch_status?: string | null
}

export type SubscriptionListResponse = {
  subscriptions: Subscription[]
}

export type CoreInfo = {
  version?: string | null
  arch?: string | null
  last_download_time?: string | null
  binary_exists: boolean
  recommended_arch: string
}

export type CoreStatus = {
  running: boolean
  pid?: number | null
}

export type Settings = {
  password_set: boolean
  subscription_auto_update?: ScheduledTaskConfig | null
  geoip_auto_update?: ScheduledTaskConfig | null
}

export type AuthLoginResponse = {
  token: string
  expires_at: number
}

export type UserProfileSummary = {
  id: string
  name: string
  is_active: boolean
  last_modified_time?: string | null
}

export type UserProfileListResponse = {
  user_profiles: UserProfileSummary[]
}

export type UserProfileDetail = {
  id: string
  name: string
  is_active: boolean
  last_modified_time?: string | null
  content: string
}

export type MergedConfig = {
  content: string
}

export type LogResponse = {
  lines: string[]
}

export type ProxyNode = {
  name: string
  type: string
  delay?: number | null
}

export type ProxyGroup = {
  name: string
  type: string
  now?: string | null
  nodes: ProxyNode[]
}

export type ProxiesView = {
  groups: ProxyGroup[]
}

export type ScheduledTaskConfig = {
  cron: string
  enabled: boolean
  last_run_time?: string | null
  last_run_status?: string | null
  last_run_message?: string | null
}
