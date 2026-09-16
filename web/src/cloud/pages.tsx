import { useEffect, useState, type ReactNode } from "react";
import {
  Link,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router-dom";
import { api, displayTime, type Resource } from "./model";
import { useWorkspace } from "./context";
import { Editor } from "./Forms";
import { ManagedProfile, ManagedSource } from "./Store";
import {
  CodeBlock,
  ConfigPreview,
  Copy,
  Empty,
  FieldActionRow,
  Icon,
  Modal,
  Panel,
  PanelBody,
  ResourceLink,
  Status,
} from "./ui";
import { resourcePath, sectionOf, sections, type Section } from "./navigation";
import { UsageCompact, UsagePanel, RefreshHistory } from "./Usage";

const meta = (section: Section) => sections.find((s) => s.key === section)!;
const host = (url?: string) => {
  try {
    return new URL(url ?? "").hostname;
  } catch {
    return "尚未设置";
  }
};
const interval = (r: Resource) =>
  r.data.auto_refresh === false
    ? "手动更新"
    : `每 ${(r.data.interval_seconds ?? 3600) / 60} 分钟`;
function Heading({
  section,
  title,
  description,
  children,
}: {
  section: Section;
  title?: string;
  description?: string;
  children?: ReactNode;
}) {
  return (
    <header className="page-heading">
      <div>
        <div className="eyebrow">{section.toUpperCase()}</div>
        <h1>{title ?? meta(section).name}</h1>
        <p>{description ?? meta(section).sub}</p>
      </div>
      <div className="heading-actions">{children}</div>
    </header>
  );
}
function Confirm({
  title,
  text,
  action,
  close,
}: {
  title: string;
  text: string;
  action: () => Promise<unknown>;
  close: () => void;
}) {
  const { busy } = useWorkspace();
  return (
    <Modal title={title} close={close}>
      <p>{text}</p>
      <div className="form-actions">
        <button disabled={busy} onClick={close}>
          取消
        </button>
        <button
          className="danger"
          disabled={busy}
          onClick={() => {
            void action().then(close);
          }}
        >
          {busy ? "处理中…" : "确认"}
        </button>
      </div>
    </Modal>
  );
}
export function CollectionPage({ section }: { section: Section }) {
  const { resources } = useWorkspace();
  const [search, setSearch] = useSearchParams();
  const query = search.get("q") ?? "";
  const all = resources.filter((r) => sectionOf(r) === section);
  const rows = all.filter((r) =>
    r.data.name.toLowerCase().includes(query.toLowerCase()),
  );
  const create = (
    <Link className="button primary" to={`/${section}/new`}>
      <Icon name="plus" size={16} />
      新建{meta(section).name}
    </Link>
  );
  const refCount = (id: string) =>
    resources.filter(
      (r) =>
        r.kind === "bundle" &&
        r.data.profiles?.some((p) => p.profile_id === id),
    ).length;
  return (
    <>
      <Heading section={section}>{create}</Heading>
      {section === "proxies" && <GlobalEgress />}
      {section === "identities" && (
        <dl className="workspace-summary" aria-label="工作区概览">
          <div>
            <dt>已发布身份</dt>
            <dd>
              <strong>
                {
                  all.filter((r) => r.data.published_revision && !r.data.error)
                    .length
                }
              </strong>
              <small>/ {all.length} 个身份</small>
            </dd>
          </div>
          <div>
            <dt>订阅源</dt>
            <dd>
              <strong>
                {
                  resources.filter(
                    (r) => r.kind === "profile" && r.data.type === "source",
                  ).length
                }
              </strong>
              <small>独立管理</small>
            </dd>
          </div>
          <div>
            <dt>绑定设备</dt>
            <dd>
              <strong>
                {
                  resources.filter(
                    (r) => r.kind === "device" && r.data.bundle_id,
                  ).length
                }
              </strong>
              <small>自动同步</small>
            </dd>
          </div>
        </dl>
      )}
      <div className="collection-bar">
        <div className="collection-label">
          全部{meta(section).name}
          <span className="count">{all.length}</span>
        </div>
        <label className="search-field">
          <Icon name="search" size={16} />
          <input
            aria-label={`搜索${meta(section).name}`}
            placeholder="搜索名称…"
            value={query}
            onChange={(e) => {
              const p = new URLSearchParams(search);
              if (e.target.value) p.set("q", e.target.value);
              else p.delete("q");
              setSearch(p, { replace: true });
            }}
          />
        </label>
      </div>
      {!rows.length ? (
        <div className="panel">
          <Empty
            title={query ? "没有匹配的结果" : `还没有${meta(section).name}`}
            text={
              query
                ? "试试其他名称，或清空搜索条件。"
                : "从创建第一项开始，逐步组织你的网络配置。"
            }
            action={!query ? create : undefined}
          />
        </div>
      ) : section === "identities" ? (
        <div className="identity-grid">
          {rows.map((r) => (
            <article className="identity-card" key={r.id}>
              <div className="card-top">
                <span className="resource-icon">
                  <Icon name="layers" size={22} />
                </span>
                <Status r={r} />
              </div>
              <h2>
                <Link to={resourcePath(r)}>{r.data.name}</Link>
              </h2>
              <p className="muted">
                {r.data.profiles?.filter((p) => p.enabled).length ?? 0}{" "}
                项启用配置 <span className="dot-separator">·</span>{" "}
                {
                  resources.filter(
                    (d) => d.kind === "device" && d.data.bundle_id === r.id,
                  ).length
                }{" "}
                台关联设备
              </p>
              <div className="profile-chips">
                {(r.data.profiles ?? []).slice(0, 3).map((p) => (
                  <span
                    className={p.enabled ? "chip" : "chip disabled"}
                    key={p.profile_id}
                  >
                    {resources.find((x) => x.id === p.profile_id)?.data.name ??
                      "已移除的 Profile"}
                  </span>
                ))}
                {(r.data.profiles?.length ?? 0) > 3 && (
                  <span className="chip">+{r.data.profiles!.length - 3}</span>
                )}
                {!r.data.profiles?.length && (
                  <span className="muted">尚未关联配置</span>
                )}
              </div>
              <UsageCompact summary={r.data.usage_summary} />
              <div className="card-footer">
                <span>版本 {r.version}</span>
                <Link className="text-link" to={resourcePath(r)}>
                  查看身份
                  <Icon name="arrow" size={15} />
                </Link>
              </div>
            </article>
          ))}
        </div>
      ) : (
        <div className="panel table-wrap">
          <table className="resource-table">
            <thead>
              <tr>
                <th>名称</th>
                <th>状态 / 类型</th>
                {section === "subscriptions" && <th>套餐使用量</th>}
                <th>
                  {section === "subscriptions"
                    ? "拉取出口"
                    : section === "profiles"
                      ? "使用情况"
                      : section === "proxies"
                        ? "使用情况"
                        : section === "devices"
                          ? "关联身份"
                          : "授权范围"}
                </th>
                <th>
                  {section === "subscriptions"
                    ? "最近刷新"
                    : section === "profiles"
                      ? "内容"
                      : section === "proxies"
                        ? "白名单 IP"
                        : section === "devices"
                          ? "最近上报"
                          : "操作"}
                </th>
                <th>
                  <span className="sr-only">查看详情</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.id}>
                  <td>
                    <Link className="row-title" to={resourcePath(r)}>
                      <span className="small-resource-icon">
                        <Icon name={meta(section).icon} />
                      </span>
                      {r.data.name}
                    </Link>
                    <small className="row-subtitle">
                      {section === "subscriptions"
                        ? host(r.data.url)
                        : section === "profiles"
                          ? r.data.store
                            ? `商店组件 · v${r.data._package?.version ?? "—"} · 手动锁定`
                            : `独立配置 · v${r.version}`
                          : section === "proxies"
                            ? r.data.provider === "xiequ"
                              ? "每次刷新即时提取"
                              : host(r.data.endpoint ?? r.data.url)
                            : `配置版本 ${r.data.reported?.revision?.slice(0, 8) ?? "未上报"}`}
                    </small>
                  </td>
                  <td>
                    {section === "proxies" ? (
                      <span className="chip">
                        {r.data.provider === "xiequ"
                          ? "携趣 · 短效"
                          : "固定代理"}
                      </span>
                    ) : (
                      <Status r={r} />
                    )}
                  </td>
                  {section === "subscriptions" && (
                    <td>
                      <UsageCompact summary={r.data.usage_summary} />
                    </td>
                  )}
                  <td>
                    {section === "subscriptions" ? (
                      "平台统一管理"
                    ) : section === "profiles" ? (
                      `${refCount(r.id)} 个身份引用`
                    ) : section === "proxies" ? (
                      "平台共享"
                    ) : (
                      <ResourceLink
                        r={resources.find((p) => p.id === r.data.bundle_id)}
                      />
                    )}
                  </td>
                  <td>
                    {section === "subscriptions" ? (
                      <>
                        {displayTime(r.data.last_fetch)}
                        <small>{interval(r)}</small>
                      </>
                    ) : section === "profiles" ? (
                      `${(r.data.content ?? "").split("\n").length} 行 YAML`
                    ) : section === "proxies" ? (
                      (r.data.whitelist_ip ?? "—")
                    ) : (
                      displayTime(r.data.reported?.seen_at)
                    )}
                  </td>
                  <td>
                    <Link
                      className="row-action"
                      aria-label={`查看 ${r.data.name}`}
                      to={resourcePath(r)}
                    >
                      <Icon name="arrow" />
                    </Link>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {section === "subscriptions" && (
        <p className="section-note">
          <Icon name="shield" size={15} />
          订阅源只负责拉取上游内容。是否参与合并，由身份中的关联开关决定。
        </p>
      )}
      {section === "profiles" && (
        <p className="section-note">
          <Icon name="code" size={15} />
          一个 Profile 可以被多个身份复用；修改后，所有关联身份会自动重新生成。
        </p>
      )}
    </>
  );
}
function GlobalEgress() {
  const { resources, busy, run } = useWorkspace();
  const [policy, setPolicy] = useState<{
    proxy_id: string | null;
    version: number;
  }>();
  const [selected, setSelected] = useState("");
  const [error, setError] = useState("");
  const [confirm, setConfirm] = useState(false);
  const reload = () => {
    void api<{ proxy_id: string | null; version: number }>(
      "/admin/subscription-egress",
    )
      .then((p) => {
        setError("");
        setPolicy(p);
        setSelected(p.proxy_id ?? "");
      })
      .catch((e: Error) => setError(e.message));
  };
  useEffect(reload, []);
  const proxies = resources.filter((r) => r.kind === "proxy");
  return (
    <Panel
      title="全局订阅出口"
      description="所有用户的首次拉取、手动和定时刷新均使用此出口。"
      actions={
        <span className="chip">
          {policy ? (policy.proxy_id ? "统一代理" : "刷新已暂停") : "读取中"}
        </span>
      }
    >
      <PanelBody>
        {error && (
          <p role="alert" className="inline-error">
            {error} <button onClick={reload}>重新读取</button>
          </p>
        )}
        <FieldActionRow>
          <label>
            生效代理
            <select
              value={selected}
              disabled={!policy || busy}
              onChange={(e) => setSelected(e.target.value)}
            >
              <option value="">暂停订阅拉取（不直连）</option>
              {proxies.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.data.name}
                </option>
              ))}
            </select>
          </label>
          <button
            className="primary"
            disabled={!policy || busy || selected === (policy.proxy_id ?? "")}
            onClick={() => setConfirm(true)}
          >
            应用出口
          </button>
        </FieldActionRow>
        <p className="muted">
          代理故障时保留上次成功配置，不会回退直连。切换不改变设备流量，也不会立即刷新全部订阅。
        </p>
      </PanelBody>
      {confirm && policy && (
        <Confirm
          title={selected ? "切换全局订阅出口？" : "暂停全平台订阅拉取？"}
          text="此操作影响所有用户的后续订阅刷新；已下发配置继续有效。在途任务将取消并重新排队。"
          close={() => setConfirm(false)}
          action={async () => {
            const next = await run(
              () =>
                api<{ proxy_id: string | null; version: number }>(
                  "/admin/subscription-egress",
                  "PUT",
                  { version: policy.version, proxy_id: selected || null },
                ),
              "全局订阅出口已更新。",
            );
            if (next) {
              setPolicy(next);
              setSelected(next.proxy_id ?? "");
            } else {
              reload();
            }
          }}
        />
      )}
    </Panel>
  );
}
function Facts({ items }: { items: [string, ReactNode][] }) {
  return (
    <dl className="facts">
      {items.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}
function References({ r }: { r: Resource }) {
  const { resources } = useWorkspace();
  const refs = resources.filter((x) =>
    r.kind === "proxy"
      ? x.data.proxy_id === r.id
      : x.kind === "bundle" &&
        x.data.profiles?.some((p) => p.profile_id === r.id),
  );
  return (
    <section className="panel">
      <div className="panel-heading">
        <h2>
          {r.kind === "proxy" ? "使用此出口的订阅源" : "使用此配置的身份"}
        </h2>
        <span className="count">{refs.length}</span>
      </div>
      {refs.length ? (
        <div className="reference-list">
          {refs.map((x) => (
            <div key={x.id}>
              <ResourceLink r={x} />
              <Status r={x} />
            </div>
          ))}
        </div>
      ) : (
        <p className="panel-message">
          尚未被引用。可以在身份编辑页中关联此配置。
        </p>
      )}
    </section>
  );
}
const outputFormats = [
  { value: "router", label: "完整 YAML" },
  { value: "clash", label: "Clash / Mihomo" },
  { value: "shadowrocket", label: "Shadowrocket 完整配置" },
  { value: "shadowrocket-nodes", label: "Shadowrocket 节点" },
];
function Distribution({ r }: { r: Resource }) {
  const [format, setFormat] = useState("router"),
    [reveal, setReveal] = useState(false);
  const url = r.data.subscription_url
    ? r.data.subscription_url + (format === "router" ? "" : `/${format}`)
    : "";
  return (
    <section className="panel distribution">
      <span className="resource-icon">
        <Icon name="radio" />
      </span>
      <h2>下发渠道 · 订阅链接</h2>
      <p className="muted">
        复制到 Clash Verge Rev 或 Shadowrocket。路由器请通过设备绑定接入。
      </p>
      <label>
        客户端格式
        <select value={format} onChange={(e) => setFormat(e.target.value)}>
          {outputFormats.map((f) => (
            <option value={f.value} key={f.value}>
              {f.label}
            </option>
          ))}
        </select>
      </label>
      {url ? (
        <>
          <label>
            身份订阅地址
            <input
              aria-label="身份订阅地址"
              type={reveal ? "text" : "password"}
              readOnly
              value={url}
            />
          </label>
          <div className="distribution-actions">
            <Copy value={url} />
            <button className="quiet" onClick={() => setReveal(!reveal)}>
              {reveal ? "隐藏" : "显示"}
            </button>
          </div>
        </>
      ) : (
        <p className="muted">当前没有订阅地址，请在「更多」中重新生成。</p>
      )}
      <div className="distribution-note">
        <Icon name="shield" size={15} />
        <span>订阅地址包含访问凭据，请勿公开分享。</span>
      </div>
      <Link className="text-link" to={`${resourcePath(r)}?tab=more`}>
        更多订阅管理 <Icon name="arrow" size={14} />
      </Link>
    </section>
  );
}
type HistoricalLink = { id: string; label: string; created_at: number };
function SubscriptionManagement({ r }: { r: Resource }) {
  const { busy, run } = useWorkspace();
  const [reset, setReset] = useState<number | null>(null);
  const [revoke, setRevoke] = useState<HistoricalLink | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [result, setResult] = useState<{
    key: string;
    links?: HistoricalLink[];
    error?: string;
  } | null>(null);
  const key = `${r.id}:${r.version}:${refresh}`;
  useEffect(() => {
    let active = true;
    void api<HistoricalLink[]>(`/bundles/${r.id}/subscription-links`)
      .then((links) => {
        if (active) setResult({ key, links });
      })
      .catch((e) => {
        if (active) setResult({ key, error: e.message });
      });
    return () => {
      active = false;
    };
  }, [key, r.id]);
  const current = result?.key === key ? result : null;
  return (
    <div className="settings-stack">
      <Panel
        title="订阅链接"
        description="一个身份，一个稳定地址。多个客户端可以共用，配置更新不会改变地址。"
      >
        <PanelBody>
          <p className="muted">
            怀疑链接泄露时可以重置。只有当前默认链接失效；历史链接和已绑定设备不受影响。
          </p>
          <div className="distribution-actions">
            {r.data.subscription_url && (
              <Copy value={r.data.subscription_url} label="复制当前链接" />
            )}
            <button
              className="danger"
              disabled={busy}
              onClick={() => setReset(r.version)}
            >
              {r.data.subscription_url ? "重置订阅链接" : "重新生成订阅链接"}
            </button>
          </div>
        </PanelBody>
      </Panel>
      <Panel
        title="历史订阅链接"
        description="此前生成的额外链接继续有效，不会自动清理。仅在确认不再使用或已经泄露时撤销。"
      >
        <PanelBody>
          {current?.error ? (
            <p className="inline-error" role="alert">
              {current.error}{" "}
              <button onClick={() => setRefresh((n) => n + 1)}>重试</button>
            </p>
          ) : !current?.links ? (
            <p className="muted" role="status">
              正在加载历史链接…
            </p>
          ) : current.links.length === 0 ? (
            <p className="muted">没有历史订阅链接。</p>
          ) : (
            <div className="reference-list">
              {current.links.map((link) => (
                <div key={link.id}>
                  <div>
                    <strong>
                      {link.label === "客户端订阅" ||
                      link.label === "Identity subscription"
                        ? "历史订阅链接"
                        : link.label}
                    </strong>
                    <p className="muted">
                      创建于 {displayTime(link.created_at)}
                    </p>
                    <small className="muted">
                      标识 {link.id.slice(0, 16)} · 非订阅密钥
                    </small>
                  </div>
                  <button
                    className="danger-text"
                    disabled={busy}
                    onClick={() => setRevoke(link)}
                  >
                    撤销
                  </button>
                </div>
              ))}
            </div>
          )}
          <p className="muted">
            系统不保存这些额外链接的可恢复明文，无法重新复制。名称相同不代表同一个链接，请勿批量撤销。
          </p>
        </PanelBody>
      </Panel>
      {reset !== null && (
        <Confirm
          title="重置订阅链接？"
          text="当前默认链接将立即失效，使用它的所有客户端都需要重新导入新地址。历史订阅链接和已绑定设备不受影响。"
          close={() => setReset(null)}
          action={async () => {
            const updated = await run(
              () =>
                api<Resource>(
                  `/bundles/${r.id}/subscription-links/reset`,
                  "POST",
                  { version: reset },
                ),
              "新订阅链接已生成，请复制并重新导入客户端。",
            );
            if (updated) setRefresh((n) => n + 1);
          }}
        />
      )}
      {revoke && (
        <Confirm
          title="撤销这条历史链接？"
          text={`标识 ${revoke.id.slice(0, 16)}。使用此链接的客户端将无法继续更新配置；当前默认链接、其他历史链接和设备不受影响。此操作不可撤销。`}
          close={() => setRevoke(null)}
          action={async () => {
            const ok = await run(async () => {
              await api(
                `/bundles/${r.id}/subscription-links/${revoke.id}`,
                "DELETE",
              );
              return true;
            }, "历史订阅链接已撤销。");
            if (ok) setRefresh((n) => n + 1);
          }}
        />
      )}
    </div>
  );
}
function Preview({ r, source = false }: { r: Resource; source?: boolean }) {
  const [params, setParams] = useSearchParams();
  const format = params.get("format") ?? "router";
  const [result, setResult] = useState<{
    key: string;
    content?: string;
    error?: string;
  } | null>(null);
  const key = `${r.id}:${r.version}:${format}:${r.data.published_revision ?? ""}`;
  useEffect(() => {
    let active = true;
    void api<{ content: string }>(
      source
        ? `/profiles/${r.id}/content`
        : `/bundles/${r.id}/preview/${format}`,
    )
      .then((v) => {
        if (active) setResult({ key, content: v.content });
      })
      .catch((e) => {
        if (active) setResult({ key, error: e.message });
      });
    return () => {
      active = false;
    };
  }, [r.id, format, key, source]);
  return (
    <ConfigPreview
      title={source ? "上次成功拉取的内容" : "合并结果"}
      description={
        source
          ? "只读快照。刷新失败不会覆盖上次成功内容。"
          : "此处为云端发布内容；客户端仍可能应用自身的本地设置。"
      }
      actions={
        !source && (
          <select
            aria-label="预览格式"
            value={format}
            onChange={(e) => {
              const p = new URLSearchParams(params);
              p.set("format", e.target.value);
              setParams(p);
            }}
          >
            {outputFormats.map((f) => (
              <option key={f.value} value={f.value}>
                {f.label}
              </option>
            ))}
          </select>
        )
      }
      loading={result?.key !== key}
      error={result?.key === key ? result.error : undefined}
      content={result?.key === key ? result.content : undefined}
    />
  );
}
function History({ r }: { r: Resource }) {
  const { run } = useWorkspace();
  const [entries, setEntries] = useState<
      { id: string; created_at: string }[] | null
    >(null),
    [error, setError] = useState(""),
    [rollback, setRollback] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    void api<{ id: string; created_at: string }[]>(`/bundles/${r.id}/revisions`)
      .then((v) => {
        if (active) setEntries(v);
      })
      .catch((e) => {
        if (active) setError(e.message);
      });
    return () => {
      active = false;
    };
  }, [r.id, r.version, r.data.published_revision]);
  return (
    <section className="panel">
      <div className="panel-heading">
        <h2>发布历史</h2>
        <span className="muted">回滚发布内容不会改写 Profile</span>
      </div>
      {error ? (
        <p className="inline-error">{error}</p>
      ) : !entries ? (
        <p className="panel-message">正在加载…</p>
      ) : !entries.length ? (
        <Empty text="首次成功发布后，版本记录将显示在这里。" />
      ) : (
        <div className="history-list">
          {entries.map((e) => (
            <div key={e.id}>
              <span className="history-marker">
                <Icon name="clock" />
              </span>
              <div>
                <strong>{e.id.slice(0, 12)}</strong>
                <small>{new Date(e.created_at).toLocaleString()}</small>
              </div>
              {e.id === r.data.published_revision ? (
                <span className="status good">当前版本</span>
              ) : (
                <button onClick={() => setRollback(e.id)}>回滚到此版本</button>
              )}
            </div>
          ))}
        </div>
      )}
      {rollback && (
        <Confirm
          title="回滚已发布配置？"
          text="客户端将收到所选历史版本的内容。下一次 Profile 变化仍会按当前组合重新生成。"
          close={() => setRollback(null)}
          action={() =>
            run(
              () =>
                api(`/bundles/${r.id}/rollback`, "POST", {
                  revision: rollback,
                }),
              "已回滚发布内容。",
            )
          }
        />
      )}
    </section>
  );
}
function Composition({ r }: { r: Resource }) {
  const { resources, busy, save } = useWorkspace();
  const bindings = r.data.profiles ?? [];
  return (
    <div className="detail-columns">
      <div className="detail-main">
        <section className="panel">
          <div className="panel-heading">
            <div>
              <h2>
                配置组合 <span className="count">{bindings.length}</span>
              </h2>
              <p className="muted">
                按以下顺序合并，后面的配置可以覆盖前面的设置。
              </p>
            </div>
            <Link className="button" to={`${resourcePath(r)}/edit`}>
              <Icon name="edit" size={15} />
              编排
            </Link>
          </div>
          {bindings.length ? (
            <div className="composition-list">
              {bindings.map((b, i) => {
                const p = resources.find((x) => x.id === b.profile_id);
                return (
                  <div
                    key={b.profile_id}
                    className={`composition-row ${!b.enabled ? "inactive" : ""}`}
                  >
                    <span className="step-number">
                      {String(i + 1).padStart(2, "0")}
                    </span>
                    <span className="small-resource-icon">
                      <Icon
                        name={p?.data.type === "source" ? "radio" : "code"}
                      />
                    </span>
                    <div className="composition-name">
                      <ResourceLink r={p} />
                      <small>
                        {p?.data.type === "source" ? "订阅源" : "配置 Profile"}
                        {p ? ` · v${p.version}` : ""}
                        {p?.data.store &&
                          ` · 商店 ${p.data._package?.version} · ${b.parameters?.policy || p.data._package?.manifest.default_policy || "未选择策略"}`}
                      </small>
                    </div>
                    <label className="switch">
                      <input
                        type="checkbox"
                        aria-label={`启用 ${p?.data.name ?? "配置"}`}
                        checked={b.enabled}
                        disabled={busy}
                        onChange={(e) => {
                          void save({
                            ...r,
                            data: {
                              ...r.data,
                              profiles: bindings.map((v, index) =>
                                index === i
                                  ? { ...v, enabled: e.target.checked }
                                  : v,
                              ),
                            },
                          });
                        }}
                      />
                      <span />
                    </label>
                  </div>
                );
              })}
            </div>
          ) : (
            <Empty
              text="关联订阅源与配置 Profile，开始生成这个身份的配置。"
              action={
                <Link className="button primary" to={`${resourcePath(r)}/edit`}>
                  添加配置
                </Link>
              }
            />
          )}
          {r.data.system_profile && (
            <div className="system-profile">
              <div>
                <strong>
                  {String(bindings.length + 1).padStart(2, "0")} ·{" "}
                  {r.data.system_profile.name}
                </strong>
                <span className="chip">系统 · 固定最后 · 不可禁用</span>
              </div>
              <p className="muted">
                最高优先级保护：规则模式，云端域名
                DIRECT。该保护不替代本地应急控制。
              </p>
              <CodeBlock content={r.data.system_profile.content} />
            </div>
          )}
          <div className="panel-bottom-note">
            <Icon name="layers" size={16} />
            启用状态仅属于当前身份，不影响其他身份。
          </div>
        </section>
        <section className="panel">
          <div className="panel-heading">
            <h2>下发渠道 · 绑定设备</h2>
          </div>
          <div className="reference-list">
            {resources
              .filter((d) => d.kind === "device" && d.data.bundle_id === r.id)
              .map((d) => (
                <div key={d.id}>
                  <ResourceLink r={d} />
                  <Status r={d} />
                </div>
              ))}
            {!resources.some(
              (d) => d.kind === "device" && d.data.bundle_id === r.id,
            ) && (
              <p className="muted">
                暂无 Camofy Agent
                设备。第三方客户端通过订阅地址接入，无需登记设备。
              </p>
            )}
          </div>
        </section>
      </div>
      <Distribution r={r} />
    </div>
  );
}
export function DetailPage({ section }: { section: Section }) {
  const [clock, setClock] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setClock(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const { id } = useParams();
  const { resources, busy, run } = useWorkspace();
  const [params, setParams] = useSearchParams();
  const navigate = useNavigate();
  const [remove, setRemove] = useState(false),
    [reveal, setReveal] = useState(false);
  const r = resources.find((x) => x.id === id && sectionOf(x) === section);
  if (!r) return <NotFound />;
  const isBundle = r.kind === "bundle",
    isSource = section === "subscriptions";
  const tabs = isBundle
    ? [
        { id: "composition", label: "配置组合" },
        { id: "preview", label: "合并预览" },
        { id: "usage", label: "套餐用量" },
        { id: "history", label: "发布历史" },
        { id: "settings", label: "设置" },
        { id: "more", label: "更多" },
      ]
    : [
        { id: "overview", label: section === "profiles" ? "配置内容" : "概览" },
        ...(r.data.store ? [{ id: "management", label: "版本与副本" }] : []),
        ...(isSource ? [{ id: "content", label: "订阅内容" }] : []),
        ...(isSource ? [{ id: "refresh-history", label: "刷新历史" }] : []),
        { id: "settings", label: "设置" },
      ];
  const tab = tabs.some((t) => t.id === params.get("tab"))
    ? params.get("tab")!
    : tabs[0].id;
  return (
    <>
      <Link className="back-link" to={`/${section}`}>
        <Icon name="back" size={16} />
        全部{meta(section).name}
      </Link>
      <Heading
        section={section}
        title={r.data.name}
        description={
          isBundle
            ? "管理配置组合与下发方式。"
            : r.data.store
              ? "商店组件 · 固定版本。源码只读，启用与出口在身份中设置。"
              : meta(section).sub
        }
      >
        <Status r={r} />
        {isSource && (
          <button
            disabled={busy}
            onClick={() => {
              void run(
                () => api(`/profiles/${r.id}/refresh`, "POST"),
                "刷新任务已提交，状态将自动更新。",
              );
            }}
          >
            <Icon name="refresh" size={16} />
            立即刷新
          </button>
        )}
        <Link className="button primary" to={`${resourcePath(r)}/edit`}>
          <Icon name="edit" size={16} />
          {r.data.store ? "编辑名称" : `编辑${isBundle ? "身份" : ""}`}
        </Link>
      </Heading>
      {r.data.error && (
        <div className="banner error">
          <div>
            <strong>
              {isSource ? "最近一次刷新失败" : "最近一次生成失败"}
            </strong>
            <span>{r.data.error}</span>
            <small>上次成功发布的内容仍然保留。</small>
          </div>
        </div>
      )}
      <div className="tabs" role="tablist" aria-label="详情导航">
        {tabs.map((t) => (
          <button
            role="tab"
            aria-selected={tab === t.id}
            key={t.id}
            className={tab === t.id ? "selected" : ""}
            onClick={() => {
              const p = new URLSearchParams();
              p.set("tab", t.id);
              setParams(p);
            }}
          >
            {t.label}
          </button>
        ))}
      </div>
      {tab === "composition" && <Composition key={r.id} r={r} />}
      {tab === "preview" && <Preview r={r} />}
      {tab === "history" && <History key={r.id} r={r} />}
      {tab === "usage" && <UsagePanel r={r} />}
      {tab === "refresh-history" && <RefreshHistory key={r.id} r={r} />}
      {tab === "content" && <Preview source r={r} />}
      {tab === "management" && r.data.store && (
        <div className="package-management">
          <ManagedProfile key={`${r.id}-${r.version}`} resource={r} />
        </div>
      )}
      {tab === "overview" && (
        <div className="detail-columns">
          <div className="detail-main">
            {isSource && <UsagePanel r={r} />}
            {section === "profiles" && r.data.store ? (
              <ManagedSource key={`${r.id}-${r.version}`} resource={r} />
            ) : section === "profiles" ? (
              <ConfigPreview
                title="独立配置"
                actions={<span className="chip">v{r.version}</span>}
                content={r.data.content ?? ""}
              />
            ) : (
              <section className="panel">
                <div className="panel-heading">
                  <h2>
                    {isSource
                      ? "订阅信息"
                      : section === "proxies"
                        ? "出口信息"
                        : "运行状态"}
                  </h2>
                </div>
                {isSource ? (
                  <>
                    <Facts
                      items={[
                        ["上游站点", host(r.data.url)],
                        ["自动刷新", interval(r)],
                        ["最近刷新", displayTime(r.data.last_fetch)],
                        ["拉取出口", "平台统一管理"],
                        ["配置版本", `v${r.version}`],
                      ]}
                    />
                    <div className="secret-field">
                      <label>
                        上游订阅 URL
                        <input
                          type={reveal ? "text" : "password"}
                          readOnly
                          value={r.data.url ?? ""}
                        />
                      </label>
                      <div className="distribution-actions">
                        <Copy value={r.data.url ?? ""} />
                        <button
                          className="quiet"
                          onClick={() => setReveal(!reveal)}
                        >
                          {reveal ? "隐藏" : "显示"}
                        </button>
                      </div>
                    </div>
                  </>
                ) : section === "proxies" ? (
                  <Facts
                    items={[
                      [
                        "供应商",
                        r.data.provider === "xiequ"
                          ? "携趣 · 短效代理"
                          : "固定代理",
                      ],
                      ["协议", r.data.protocol ?? "由 URL 决定"],
                      ["固定出口", r.data.endpoint ?? "—"],
                      ["白名单 IP", r.data.whitelist_ip ?? "—"],
                      ["白名单确认", displayTime(r.data.whitelist_at)],
                    ]}
                  />
                ) : (
                  <>
                    <Facts
                      items={[
                        [
                          "身份",
                          <ResourceLink
                            r={resources.find((x) => x.id === r.data.bundle_id)}
                          />,
                        ],
                        ["应用状态", <Status r={r} />],
                        ["最近上报", displayTime(r.data.reported?.seen_at)],
                        [
                          "已应用版本",
                          r.data.reported?.revision?.slice(0, 12) ?? "—",
                        ],
                        ["反馈", r.data.reported?.message ?? "—"],
                        [
                          "Mihomo",
                          (
                            {
                              running: "运行中",
                              stopped: "已停止",
                              unavailable: "未就绪",
                            } as Record<string, string>
                          )[r.data.reported?.core_state ?? ""] ??
                            "等待设备上报",
                        ],
                        ["控制结果", r.data.reported?.command_error || "—"],
                        [
                          "待执行指令",
                          r.data.command
                            ? r.data.command.expires_at * 1000 > clock
                              ? `${r.data.command.type} · 等待回执`
                              : "已过期，未获回执"
                            : "无",
                        ],
                      ]}
                    />
                    <div className="panel-actions">
                      {(
                        [
                          ["start", "启动"],
                          ["stop", "停止"],
                          ["restart", "重启"],
                        ] as const
                      ).map(([action, label]) => (
                        <button
                          key={action}
                          disabled={
                            busy ||
                            !!(
                              r.data.command &&
                              r.data.command.expires_at * 1000 > clock
                            )
                          }
                          onClick={() => {
                            if (
                              window.confirm(
                                `${label}此设备的 Mihomo？可能影响设备网络；可通过路由器本地页面恢复。`,
                              )
                            )
                              void run(
                                () =>
                                  api(`/devices/${r.id}/control`, "POST", {
                                    action,
                                  }),
                                `${label}指令已发送，等待设备执行回执（两分钟内有效）。`,
                              );
                          }}
                        >
                          {label} Mihomo
                        </button>
                      ))}
                      <button
                        disabled={busy}
                        onClick={() => {
                          void run(
                            () => api(`/devices/${r.id}/test`, "POST"),
                            "测速指令已发送，等待设备上报。",
                          );
                        }}
                      >
                        请求节点测速
                      </button>
                    </div>
                    {r.data.reported?.delays && (
                      <Facts
                        items={Object.entries(r.data.reported.delays).map(
                          ([name, delay]) => [
                            name,
                            delay === null ? "超时" : `${delay} ms`,
                          ],
                        )}
                      />
                    )}
                  </>
                )}
              </section>
            )}
            {section !== "devices" && section !== "proxies" && (
              <References r={r} />
            )}
          </div>
          <aside className="panel guidance">
            <Icon name={meta(section).icon} size={24} />
            <h2>
              {isSource
                ? "上游内容，独立管理"
                : section === "profiles"
                  ? "可复用的配置单元"
                  : section === "proxies"
                    ? "拉取流量的出口"
                    : "设备保持轻量"}
            </h2>
            <p>
              {isSource
                ? "定时刷新与手动刷新使用相同的拉取代理。刷新成功后，引用此订阅的身份会自动重新生成。"
                : section === "profiles"
                  ? "为节点、代理组、域名规则或运行参数分别建立 Profile，然后按用途自由组合。"
                  : section === "proxies"
                    ? "此代理仅用于云端拉取上游订阅，不会成为设备的流量出口。携趣每次刷新即时提取一个短效 IP。"
                    : "身份由云端直接分配，设备自动同步，无需配置订阅 URL。停止状态会持久保存，不会被同步或看门狗重新拉起；云端不可达时，请通过路由器本地页面启动、停止或重启 Mihomo。"}
            </p>
            <div className="guidance-rule" />
            <p className="muted">
              {section === "profiles"
                ? "支持 prepend- / append- 合并指令。启用或禁用配置，请在身份中操作。"
                : "敏感链接与凭据请妥善保管，不要公开分享。"}
            </p>
          </aside>
        </div>
      )}
      {tab === "more" && isBundle && (
        <SubscriptionManagement key={r.id} r={r} />
      )}
      {tab === "settings" && (
        <div className="settings-stack">
          <section className="panel">
            <div className="panel-heading">
              <h2>基本信息</h2>
            </div>
            <Facts
              items={[
                ["名称", r.data.name],
                ["类型", meta(section).name],
                ["版本", `v${r.version}`],
                ["资源 ID", <code className="break-anywhere">{r.id}</code>],
              ]}
            />
          </section>
          <section className="panel danger-zone">
            <div>
              <h2>
                {r.kind === "device" ? "解绑设备" : `删除${meta(section).name}`}
              </h2>
              <p>
                {r.kind === "device"
                  ? "解绑会移除设备记录并撤销云端授权；不停止本地 Mihomo，设备会保留最后有效配置。重新连接需要再次授权。"
                  : "删除不可撤销。被其他资源引用时，请先解除关联。"}
              </p>
            </div>
            <button
              className="danger"
              disabled={busy}
              onClick={() => setRemove(true)}
            >
              {r.kind === "device" ? "解绑" : "删除"}
            </button>
          </section>
        </div>
      )}
      {remove && (
        <Confirm
          title={`${r.kind === "device" ? "解绑" : "删除"}「${r.data.name}」？`}
          text={
            r.kind === "device"
              ? "设备将失去云端同步与远程控制授权，但本地 Mihomo 不会因此停止。身份订阅链接和其他设备不受影响。"
              : isBundle
                ? "此操作不可撤销。身份删除后，使用该身份的订阅链接将失效。"
                : "此操作不可撤销。被其他资源引用时，请先解除关联。"
          }
          close={() => setRemove(false)}
          action={async () => {
            const result = await run(async () => {
              await api(
                r.kind === "proxy"
                  ? `/admin/proxies/${r.id}`
                  : `/resources/${r.id}`,
                "DELETE",
              );
              return true;
            }, "已删除。");
            if (result) navigate(`/${section}`);
          }}
        />
      )}
    </>
  );
}
export function EditPage({
  section,
  fresh = false,
}: {
  section: Section;
  fresh?: boolean;
}) {
  const { id } = useParams();
  const { resources, busy, error, save } = useWorkspace();
  const navigate = useNavigate();
  const existing = resources.find(
    (r) => r.id === id && sectionOf(r) === section,
  );
  if (!fresh && !existing) return <NotFound />;
  const initial: Resource = existing ?? {
    id: "",
    version: 0,
    kind:
      section === "identities"
        ? "bundle"
        : section === "proxies"
          ? "proxy"
          : section === "devices"
            ? "device"
            : "profile",
    data: {
      name: "",
      ...(section === "subscriptions"
        ? {
            type: "source",
            url: "",
            auto_refresh: true,
            interval_seconds: 3600,
          }
        : section === "profiles"
          ? {
              type: "overlay",
              content: "# 添加你的节点、规则或运行参数\nprepend-rules: []\n",
            }
          : section === "identities"
            ? { profiles: [], selections: {} }
            : section === "proxies"
              ? { provider: "static" as const, url: "" }
              : { bundle_id: "" }),
    },
  };
  const back = existing ? resourcePath(existing) : `/${section}`;
  return (
    <>
      <Link className="back-link" to={back}>
        <Icon name="back" size={16} />
        返回{existing ? "详情" : meta(section).name}
      </Link>
      <Heading
        section={section}
        title={`${fresh ? "新建" : "编辑"}${meta(section).name}`}
        description={existing ? existing.data.name : meta(section).sub}
      />
      <div className="edit-layout">
        <Editor
          key={`${section}:${id ?? "new"}`}
          resource={initial}
          all={resources}
          busy={busy}
          serverError={error}
          onSave={save}
          onSaved={(r) => navigate(resourcePath(r))}
          onClose={() => navigate(back)}
        />
        <aside className="panel guidance">
          <span className="eyebrow">GOOD TO KNOW</span>
          <h2>
            {section === "identities" ? "先组合，再分发" : "保持配置职责清晰"}
          </h2>
          <p>
            {section === "identities"
              ? "启用开关属于此身份中的关联。一个 Profile 可以在不同身份中采用不同的启用状态。"
              : section === "subscriptions"
                ? "填写机场提供的 Clash YAML 地址，拉取出口由平台统一管理。源内容更新后，关联身份自动重新生成。"
                : section === "profiles"
                  ? "建议每份 Profile 负责一个用途，例如自定义规则、专用节点或关闭 TUN，便于复用和排查。"
                  : section === "proxies"
                    ? "此配置由管理员共同管理，供全平台订阅拉取使用。保存后还需在出口列表中选为全局生效代理。"
                    : "配置将保存在当前工作区，与其他账号隔离。"}
          </p>
          <div className="guidance-rule" />
          <p className="muted">
            保存前，未完成的修改只留在本页。离开页面时会提醒你确认。
          </p>
        </aside>
      </div>
    </>
  );
}
export function NotFound() {
  return (
    <div className="panel">
      <Empty
        title="没有找到这个页面"
        text="资源可能已删除，或链接地址有误。"
        action={
          <Link className="button primary" to="/identities">
            返回身份列表
          </Link>
        }
      />
    </div>
  );
}
