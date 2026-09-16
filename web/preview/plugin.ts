import type { Plugin } from "vite";

// Local-only visual review fixtures. No network calls, secrets, or production data.
export function designPreview(): Plugin {
  const now = Math.floor(Date.now() / 1000);
  const yaml =
    "mixed-port: 7890\nmode: rule\nallow-lan: false\ntun:\n  enable: false\nproxies: []\nproxy-groups: []\nrules:\n  - DOMAIN-SUFFIX,camofy.app,DIRECT\n  - MATCH,DIRECT\n";
  const usage = (download: number, total = 200) => ({
    status: "ok",
    known_pools: 1,
    total_pools: 1,
    upload: "2147483648",
    download: String(download * 1073741824),
    total: String(total * 1073741824),
    remaining: String((total - download - 2) * 1073741824),
    expire: now + 30 * 86400,
    next_expire: now + 30 * 86400,
    updated_at: now - 120,
    pools: [],
  });
  const source = (id: string, name: string, download: number) => ({
    id,
    kind: "profile",
    version: 4,
    data: {
      name,
      type: "source",
      url: "https://subscription.example.invalid/" + id,
      auto_refresh: true,
      interval_seconds: 3600,
      last_fetch: now - 120,
      fetch_status: "ok",
      content: yaml,
      usage_summary: usage(download),
    },
  });
  const overlay = (id: string, name: string, content: string) => ({
    id,
    kind: "profile",
    version: 2,
    data: { name, type: "overlay", content },
  });
  const identity = (
    id: string,
    name: string,
    profiles: string[],
    download: number,
  ) => ({
    id,
    kind: "bundle",
    version: 8,
    data: {
      name,
      profiles: profiles.map((profile_id) => ({ profile_id, enabled: true })),
      published_revision: "demo-revision-08",
      subscription_url: "http://127.0.0.1:18742/sub/demo-" + id,
      usage_summary: usage(download),
      system_profile: {
        name: "云端连接保护",
        locked: true,
        content:
          "mode: rule\nprepend-rules:\n  - DOMAIN-SUFFIX,camofy.app,DIRECT",
      },
    },
  });
  type Item = {
    id: string;
    kind: string;
    version: number;
    data: Record<string, unknown>;
  };
  let resources: Item[] = [
    source("everyday-source", "日常订阅", 36),
    source("backup-source", "备用线路", 8),
    {
      ...source("travel-source", "旅行订阅", 12),
      data: {
        ...source("travel-source", "旅行订阅", 12).data,
        fetch_status: "error",
        error: "订阅源连接超时，已保留上次成功配置。",
      },
    },
    overlay(
      "routing",
      "日常分流",
      "prepend-rules:\n  - DOMAIN-SUFFIX,example.org,DIRECT\n  - DOMAIN-SUFFIX,example.net,DIRECT\n",
    ),
    overlay(
      "private-nodes",
      "自建节点",
      "# 在此添加你的自建代理节点\nprepend-proxies: []\n",
    ),
    overlay("no-tun", "关闭 TUN", "tun:\n  enable: false\n"),
    identity(
      "everyday",
      "日常网络",
      ["everyday-source", "routing", "private-nodes"],
      36,
    ),
    identity("home", "家里的网络", ["backup-source", "routing", "no-tun"], 8),
    identity("travel", "轻装出行", ["travel-source", "routing"], 12),
    {
      id: "home-router",
      kind: "device",
      version: 3,
      data: {
        name: "客厅路由器",
        bundle_id: "home",
        reported: {
          status: "applied",
          revision: "demo-revision-08",
          seen_at: now - 30,
          core_state: "running",
          message: "配置已应用 · TUN 未启用",
        },
      },
    },
    {
      id: "study-router",
      kind: "device",
      version: 1,
      data: {
        name: "书房路由器",
        bundle_id: "everyday",
        reported: {
          status: "applied",
          revision: "demo-revision-08",
          seen_at: now - 90,
          core_state: "stopped",
        },
      },
    },
  ];
  const user = {
    email: "demo@example.invalid",
    nickname: "我的工作区",
    role: "user",
  };
  const packages = [
    ["daily-direct", "日常直连", "常用国内服务，使用直连策略。", "分流规则"],
    [
      "quiet-browsing",
      "清爽浏览",
      "独立维护过滤规则，按身份启用。",
      "隐私保护",
    ],
    ["developer", "开发工具", "为代码托管与包管理服务指定策略。", "开发工具"],
  ].map(([slug, name, summary, category]) => ({
    slug,
    id: "demo-" + slug,
    publisher: "本地演示",
    version: "1.0.0",
    hash: "local-preview-only",
    manifest: {
      name,
      summary,
      category,
      notes: "虚构的本地设计预览条目，不包含实际第三方规则。",
      default_policy: "DIRECT",
      sources: [],
    },
    rules: [{ kind: "DOMAIN-SUFFIX", value: "example.org", no_resolve: false }],
  }));
  return {
    name: "camofy-local-design-preview",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use(async (req, res, next) => {
        const url = new URL(req.url ?? "/", "http://localhost");
        if (!url.pathname.startsWith("/api/")) return next();
        // This mode must never fall through to a live backend.
        const send = (value: unknown, status = 200) => {
          res.statusCode = status;
          res.setHeader("Content-Type", "application/json");
          res.setHeader("Cache-Control", "no-store");
          res.end(JSON.stringify(value));
        };
        try {
          await new Promise((resolve) => setTimeout(resolve, 180));
          const path = url.pathname.slice(4);
          if (path === "/auth/logout") {
            res.setHeader(
              "Set-Cookie",
              "camofy_demo_out=1; Path=/; SameSite=Strict",
            );
            return send({});
          }
          if (path === "/auth/login" || path === "/auth/register") {
            res.setHeader(
              "Set-Cookie",
              "camofy_demo_out=; Path=/; Max-Age=0; SameSite=Strict",
            );
            return send(user);
          }
          if (path === "/auth/me")
            return req.headers.cookie?.includes("camofy_demo_out=1")
              ? send({ error: "本地演示：请登录" }, 401)
              : send(user);
          if (path === "/resources" && req.method === "GET")
            return send(resources);
          let body = "";
          for await (const chunk of req) body += chunk;
          const draft = body ? JSON.parse(body) : {};
          if (path === "/account") {
            user.nickname = draft.nickname ?? user.nickname;
            return send(user);
          }
          if (path === "/resources" && req.method === "POST") {
            const item = {
              id: "local-" + Date.now(),
              kind: draft.kind,
              version: 1,
              data: draft.data,
            };
            resources.push(item);
            return send(item);
          }
          const resourceId = path.split("/")[2];
          const current = resources.find((r) => r.id === resourceId);
          if (path.startsWith("/resources/") && current) {
            if (req.method === "DELETE") {
              resources = resources.filter((r) => r !== current);
              return send({});
            }
            if (req.method === "PUT") {
              current.data = draft.data;
              current.version++;
            }
            return send(current);
          }
          if (path.endsWith("/content") || path.includes("/preview/"))
            return send({ content: current?.data.content ?? yaml });
          if (path.endsWith("/subscription-links")) return send([]);
          if (path.endsWith("/revisions"))
            return send([
              {
                id: "demo-revision-08",
                created_at: new Date(now * 1000).toISOString(),
              },
            ]);
          if (path.endsWith("/history"))
            return send({
              next_cursor: null,
              items: [
                {
                  id: "refresh-1",
                  started_at: now - 120,
                  finished_at: now - 119,
                  duration_ms: 1200,
                  reason: "scheduled",
                  status: "ok",
                  error_code: null,
                  message: null,
                  usage_status: "ok",
                },
              ],
            });
          if (path.endsWith("/refresh") && current) {
            current.data.fetch_status = "ok";
            delete current.data.error;
            current.data.last_fetch = Math.floor(Date.now() / 1000);
            return send({});
          }
          if (path.endsWith("/control") && current) {
            current.data.reported = {
              ...(current.data.reported as object),
              core_state: draft.action === "stop" ? "stopped" : "running",
            };
            return send({});
          }
          if (path === "/store/packages") return send(packages);
          if (path.startsWith("/store/packages/")) {
            const entry = packages.find((p) => p.slug === path.split("/")[3]);
            if (entry)
              return send({
                slug: entry.slug,
                publisher: entry.publisher,
                owned: false,
                versions: [entry],
              });
          }
          return send(
            { error: "本地演示未模拟此操作；不会请求线上服务。" },
            400,
          );
        } catch {
          send({ error: "本地演示请求无效。" }, 400);
        }
      });
    },
  };
}
