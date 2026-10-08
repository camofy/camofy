import type { Plugin } from "vite";
import { readFileSync } from "node:fs";

// Local-only visual review fixtures. No network calls, secrets, or production data.
export function designPreview(): Plugin {
  const matrix = JSON.parse(readFileSync(new URL("../../src/compatibility/registry.json", import.meta.url), "utf8"));
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
  // A source's own link: a hidden identity that binds only that source.
  const sourceLink = (source: string, name: string, download: number) => {
    const link = identity("link-" + source, name, [source], download);
    return { ...link, data: { ...link.data, managed_source: source } };
  };
  type Item = {
    id: string;
    kind: string;
    version: number;
    data: Record<string, unknown>;
  };
  let resources: Item[] = [
    source("everyday-source", "日常订阅", 36),
    {
      ...source("panel-source", "机房面板订阅", 12),
      data: {
        ...source("panel-source", "机房面板订阅", 12).data,
        url: "https://panel.example.invalid/generated/clash.yaml",
        westdata: {
          username: "demo@example.invalid",
          product_id: "123456",
          subscription_url: "https://panel.example.invalid/subscribe/demo-token",
          last_activate: now - 60,
        },
      },
    },
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
    sourceLink("everyday-source", "日常订阅", 36),
    sourceLink("panel-source", "机房面板订阅", 12),
    {
      ...sourceLink("travel-source", "旅行订阅", 12),
      data: {
        ...sourceLink("travel-source", "旅行订阅", 12).data,
        error: "profile \"旅行订阅\" 的代理组引用了不存在的节点「香港 03」",
      },
    },
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
    {
      id: "balcony-router",
      kind: "device",
      version: 1,
      data: {
        name: "阳台路由器",
        bundle_id: "link-panel-source",
        reported: {
          status: "applied",
          revision: "demo-revision-08",
          seen_at: now - 45,
          core_state: "running",
        },
      },
    },
    {
      id: "demo-fanproxy",
      kind: "proxy",
      version: 1,
      data: {
        name: "网帆演示出口",
        provider: "fanproxy",
        endpoint: "网帆 · 国内短效代理 · 每次刷新提取 1 IP",
        protocol: "http",
        area: "",
        isp: "",
        deduplicate: true,
        whitelist_ip: "203.0.113.10",
        whitelist_at: now - 3600,
      },
    },
  ];
  const assistantSessions = new Map<string, { id: string; current_draft?: string; events: { type: string; text?: string; name?: string; draft_id?: string; lines?: number }[] }>();
  const assistantDrafts = new Map<string, { id: string; status: string; original: string; content: string; stale: boolean; affected: { id: string; name: string }[]; errors: unknown[]; validation_hash: string; can_commit: boolean }>();
  const user = {
    email: "demo@example.invalid",
    nickname: "我的工作区",
    role: "admin",
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
      sources: [
        {
          url: "https://example.invalid/rules/" + slug + ".list",
          revision: "0f3c9a1d2b7e4c58a6d0e1f2a3b4c5d6e7f80912",
          license: "CC0-1.0",
          license_text:
            "本地设计预览的虚构许可文本。\n正式目录中，这里展示来源仓库随版本固定的许可全文。",
          attribution: "Camofy 本地演示",
          sha256:
            "9b2f3c6e1d8a7f40b5c2e9d1a6f3b8c4e7d2a5f9c1b6e3d8a4f7c2b9e5d1a6f3",
        },
      ],
    },
    rules: [{ kind: "DOMAIN-SUFFIX", value: "example.org", no_resolve: false }],
  }));
  resources.splice(7, 0, {
    id: "store-daily-direct",
    kind: "profile",
    version: 1,
    data: {
      name: "日常直连",
      type: "overlay",
      origin: "store",
      content: "",
      store: {
        slug: "daily-direct",
        version_id: "demo-daily-direct",
        update_policy: "manual",
      },
      _package: { version: "1.0.0", manifest: packages[0].manifest },
    },
  });
  const nodes = [
    "香港 01", "香港 02", "日本 东京 01", "日本 大阪 02", "新加坡 01",
    "美国 洛杉矶 01", "美国 圣何塞 02", "台湾 01", "英国 伦敦 01",
  ];
  const proxyView = (id: string) => {
    const device = resources.find((r) => r.id === id)?.kind === "device";
    return {
      identity_id: device ? "home" : id,
      identity_name: device ? "家里的网络" : "日常网络",
      version: 3,
      groups: [
        { name: "节点选择", kind: "Selector", members: ["自动选择", ...nodes], now: "香港 01", dynamic: false },
        { name: "自动选择", kind: "URLTest", members: nodes, now: "日本 东京 01", dynamic: true },
        { name: "流媒体", kind: "Selector", members: ["节点选择", ...nodes.slice(2, 7)], now: "新加坡 01", dynamic: false },
        { name: "开发工具", kind: "Selector", members: ["DIRECT", "节点选择"], now: "节点选择", dynamic: false },
      ],
      selections: { 节点选择: "香港 01", 流媒体: "新加坡 01" },
      overrides: device ? { 流媒体: "日本 大阪 02" } : {},
      state: { status: "applied", received_at: now - 40, sampled_at: now - 45, selection_version: 3 },
      reported: device ? { protocol: 2, core_state: "running" } : undefined,
      jobs: device
        ? nodes.slice(0, 5).map((name, i) => ({
            id: "job-" + i, method: "proxies.delay", status: i === 3 ? "failed" : "succeeded",
            created_at: now - 300, params: { name },
            result: i === 3 ? { error: "timeout" } : { value: { name, delay: 48 + i * 37, sampled_at: now - 290 } },
          }))
        : [],
      events: [
        { id: "ev-2", created_at: now - 600, group: "流媒体", from: "日本 东京 01", to: "新加坡 01", source: "cloud", status: "applied" },
        { id: "ev-1", created_at: now - 7200, group: "节点选择", from: "自动选择", to: "香港 01", source: "cloud", status: "applied" },
      ],
      devices: device
        ? undefined
        : [{ id: "study-router", name: "书房路由器", reported: { proxy_state: { status: "applied", received_at: now - 40 } } }],
    };
  };
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
          if (path === "/admin/subscription-egress")
            return send({ proxy_id: null, direct: true, version: 1 });
          if (path === "/admin/proxies/egress-preview")
            return send({ ip: "203.0.113.10", expires_at: now + 300,
              proof: "local-preview-only", sources: ["local-design-fixture"] });
          if (path === "/profiles/westdata-services")
            return send({
              services: [
                {
                  id: "123456",
                  name: "Plan Beta",
                  status: "有效的",
                  next_due: "2026-10-08",
                },
                {
                  id: "654321",
                  name: "Plan Beta",
                  status: "已终止",
                  next_due: "2025-09-06",
                },
              ],
            });
          let body = "";
          for await (const chunk of req) body += chunk;
          const draft = body ? JSON.parse(body) : {};
          if (path === "/client-compatibility") return send(matrix);
          if (path.endsWith("/compatibility-preview")) {
            const identity = resources.find(r => r.id === path.split("/")[2]);
            const policy = identity?.data.node_filter as { auto: boolean; exclude_types: string[] } | undefined;
            const old = /ClashMetaForAndroid\/2\.10\.2/.test(draft.user_agent ?? "");
            const fixture = [{ name: "常用节点", protocol: "ss" }, { name: "新协议节点", protocol: "mieru" }, { name: "备用节点", protocol: "anytls" }];
            const excluded = fixture.filter(n => policy?.exclude_types.includes(n.protocol) || (policy?.auto !== false && old && n.protocol !== "ss"));
            if (excluded.length === fixture.length) return send({ error: "过滤后没有可用节点，已阻止下发空订阅；请调整该身份的过滤设置或订阅源" }, 422);
            const retained = fixture.filter(n => !excluded.includes(n));
            return send({ content: "# 虚构的本地设计预览\nproxies:\n" + retained.map(n => `  - {name: ${n.name}, type: ${n.protocol}, server: proxy.example, port: 443}`).join("\n") + "\nproxy-groups:\n  - name: 节点选择\n    type: select\n    proxies: [" + retained.map(n => n.name).join(", ") + "]\nrules: ['MATCH,节点选择']\n", report: {
              client: { name: old ? "Clash Meta for Android" : undefined, version: old ? "2.10.2" : undefined, confidence: old ? "bundled" : "unknown" },
              before: 3, retained: retained.length, removed: excluded.length, repaired_references: excluded.length,
              warnings: old ? [] : ["客户端或版本能力未确认，自动模式保留未知节点；指定类型排除仍然生效。"], unknown_capabilities: [], blocked_groups: [],
              exclusions: excluded.map(n => ({ ...n, reason: policy?.exclude_types.includes(n.protocol) ? "manual" : "unsupported_protocol", capability: n.protocol }))
            } });
          }
          if (path === "/assistant/config")
            return send({ enabled: true, model: "gpt-6-luna", effort: "medium", portal_url: "https://example.invalid/models" });
          if (path.startsWith("/profiles/") && path.endsWith("/assistant/sessions") && req.method === "POST") {
            const id = `demo-agent-${Date.now()}`;
            assistantSessions.set(id, { id, events: [] });
            return send({ id, profile_id: path.split("/")[2], model: "gpt-6-luna", effort: "medium" });
          }
          if (path.startsWith("/assistant/sessions/")) {
            const id = path.split("/")[3];
            const session = assistantSessions.get(id);
            if (!session) return send({ error: "演示对话不存在" }, 404);
            if (path.endsWith("/turn") && req.method === "POST") {
              const original = String(resources.find((r) => r.id === "routing")?.data.content ?? "");
              const content = original.replace("example.org", "example.edu");
              const draftId = `demo-draft-${Date.now()}`;
              assistantDrafts.set(draftId, { id: draftId, status: "draft", original, content, stale: false,
                affected: [{ id: "everyday", name: "日常网络" }, { id: "home", name: "家里的网络" }],
                errors: [], validation_hash: "demo-validation", can_commit: true });
              session.current_draft = draftId;
              session.events.push({ type: "user", text: draft.text }, { type: "tool", name: "profile_read", lines: 4 },
                { type: "tool", name: "profile_replace", draft_id: draftId },
                { type: "assistant", text: "已将 example.org 替换为 example.edu，生成草稿。请检查差异和受影响身份后确认提交。" });
              res.statusCode = 200;
              res.setHeader("Content-Type", "text/event-stream");
              res.setHeader("Cache-Control", "no-store");
              for (const item of [
                { type: "delta", text: "已阅读配置并生成草稿。" },
                { type: "tool", name: "profile_replace", draft_id: draftId },
                { type: "done" },
              ]) res.write(`data: ${JSON.stringify(item)}\n\n`);
              res.end(); return;
            }
            return send(session);
          }
          if (path.startsWith("/assistant/drafts/")) {
            const id = path.split("/")[3];
            const item = assistantDrafts.get(id);
            if (!item) return send({ error: "演示草稿不存在" }, 404);
            if (path.endsWith("/commit") && req.method === "POST") {
              if (draft.validation_hash !== item.validation_hash) return send({ error: "预览已过期" }, 409);
              item.status = "committed"; item.can_commit = false;
              const profile = resources.find((r) => r.id === "routing");
              if (profile) { profile.data.content = item.content; profile.version++; }
              return send({ draft_id: id, status: "committed", published_identities: item.affected, devices_applied: false });
            }
            return send(item);
          }
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
            if (item.kind === "profile" && item.data.type === "source") {
              const link = sourceLink(item.id, String(item.data.name), 0);
              resources.push({
                ...link,
                data: { ...link.data, published_revision: undefined },
              });
            }
            return send(item);
          }
          if (path.endsWith("/subscription-link") && req.method === "POST") {
            const source = resources.find((r) => r.id === path.split("/")[2]);
            if (!source) return send({ error: "演示订阅源不存在" }, 404);
            let link = resources.find((r) => r.data.managed_source === source.id);
            if (!link) {
              link = sourceLink(source.id, String(source.data.name), 0);
              resources.push(link);
            }
            return send(link);
          }
          if (path.endsWith("/promote") && req.method === "POST") {
            const link = resources.find((r) => r.id === path.split("/")[2]);
            if (!link?.data.managed_source) return send({ error: "演示身份不存在" }, 404);
            delete link.data.managed_source;
            link.version++;
            return send(link);
          }
          if (path === "/admin/proxies" && req.method === "POST") {
            const data = { ...draft.data };
            if (data.provider === "fanproxy") {
              data.endpoint = "网帆 · 国内短效代理 · 每次刷新提取 1 IP";
              data.protocol = "http";
              data.whitelist_ip = "203.0.113.10";
              data.whitelist_at = now;
              delete data.extract_key;
              delete data.whitelist_account;
              delete data.whitelist_signature;
              delete data.egress_preview;
            }
            const item = { id: "local-proxy-" + Date.now(), kind: "proxy",
              version: 1, data };
            resources.push(item);
            return send(item);
          }
          if (path.startsWith("/oauth/requests/"))
            return send({
              device_name: "客厅路由器",
              return_uri: "http://192.0.2.1:3000/cloud/callback",
              scope: "sync",
            });
          if (path.endsWith("/source-preview"))
            return send({
              content:
                "# 日常直连 v1.0.0 · 本地演示\nprepend-rules:\n  - DOMAIN-SUFFIX,example.org," +
                (draft.policy || "DIRECT") +
                "\n",
              policy: draft.policy || "DIRECT",
              parameterized: false,
            });
          if (path.endsWith("/proxies") && path.startsWith("/resources/"))
            return send(proxyView(path.split("/")[2]));
          const resourceId = path.split("/")[2];
          const current = resources.find((r) => r.id === resourceId);
          if (path.startsWith("/admin/proxies/") && current) {
            if (req.method === "PUT") {
              current.data = draft.data;
              current.version++;
            }
            return send(current);
          }
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
