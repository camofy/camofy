export type Data = {
  name: string;
  type?: string;
  url?: string;
  endpoint?: string;
  provider?: "static" | "xiequ";
  extract_url?: string;
  whitelist_uid?: string;
  whitelist_key?: string;
  whitelist_ip?: string;
  whitelist_at?: number;
  protocol?: "http" | "socks5";
  egress_preview?: unknown;
  last_proxy?: string;
  last_proxy_at?: number;
  content?: string;
  proxy_id?: string | null;
  interval_seconds?: number;
  auto_refresh?: boolean;
  usage_pool?: string;
  usage_summary?: UsageSummary;
  profiles?: { profile_id: string; enabled: boolean }[];
  subscription_url?: string;
  selections?: Record<string, string>;
  bundle_id?: string;
  published_revision?: string;
  error?: string;
  fetch_status?: string;
  last_fetch?: number;
  outputs?: Record<string, { error?: string }>;
  system_profile?: { name: string; content: string; locked: boolean };
  reported?: {
    status?: string;
    revision?: string;
    seen_at?: number;
    message?: string;
    delays?: Record<string, number | null>;
    core_state?: string;
    command_error?: string;
  };
  command?: { id: string; type: string; expires_at: number } | null;
};
export type Resource = {
  id: string;
  kind: string;
  version: number;
  data: Data;
};
export type UsagePool = {
  profile_ids: string[]; names: string[]; status: string;
  upload: string | null; download: string | null; total: string | null;
  expire: number | null; updated_at: number | null; expired: boolean; stale: boolean;
};
export type UsageSummary = {
  status: string; known_pools: number; total_pools: number;
  upload: string; download: string; total: string; remaining: string;
  expire: number | null; next_expire: number | null; updated_at: number | null;
  pools: UsagePool[];
};
export type Token = {
  id: string;
  label: string;
  bundle_id: string;
  device_id?: string;
};
export type Issued = {
  token: string;
  cloud_url: string;
  subscription_base: string;
  device_id?: string;
};
export type User = { email: string };
export const formats = ["clash", "shadowrocket", "shadowrocket-nodes"];
export const labels: Record<string, string> = {
  profile: "配置 Profile",
  bundle: "身份",
  proxy: "拉取代理",
  device: "设备",
  token: "订阅链接",
};

export async function api<T>(
  path: string,
  method = "GET",
  data?: unknown,
): Promise<T> {
  const r = await fetch(`/api${path}`, {
    method,
    credentials: "same-origin",
    headers: data === undefined ? {} : { "Content-Type": "application/json" },
    body: data === undefined ? undefined : JSON.stringify(data),
  });
  if (!r.ok) {
    const body = await r.json().catch(() => ({ error: `HTTP ${r.status}` }));
    throw new Error(body.error ?? `HTTP ${r.status}`);
  }
  return r.status === 204 || r.status === 202 ? (undefined as T) : r.json();
}
export const displayTime = (t?: number) =>
  t ? new Date(t * 1000).toLocaleString() : "—";
