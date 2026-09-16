import { useEffect, useState } from "react";
import {
  Link,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router-dom";
import { api, type Resource, type User } from "./model";
import { useWorkspace } from "./context";
import {
  Copy,
  Icon,
  Panel,
  PanelBody,
  FieldActionRow,
  ConfigPreview,
} from "./ui";
import "./store.css";

export type PackageRule = { kind: string; value: string; no_resolve: boolean };
export type PackageManifest = {
  name: string;
  summary: string;
  category: string;
  notes: string;
  default_policy: string | null;
  sources: {
    url: string;
    revision: string;
    license: string;
    license_text: string;
    attribution: string;
    sha256: string;
  }[];
};
export type PackageVersion = {
  id: string;
  version: string;
  manifest: PackageManifest;
  rules: PackageRule[];
  hash: string;
};
type Entry = PackageVersion & { slug: string; publisher: string };
type Detail = {
  slug: string;
  publisher: string;
  owned: boolean;
  versions: PackageVersion[];
};
type Upgrade = {
  preview_digest: string;
  added: number;
  removed: number;
  affected: {
    id: string;
    name: string;
    warnings: string[];
    outputs: Record<string, { error?: string }>;
  }[];
};
export type IdentityPreview = {
  artifacts: Record<string, { content?: string; error?: string }>;
  warnings: string[];
  policies: string[];
};

export function StorePage() {
  const [items, setItems] = useState<Entry[] | null>(null),
    [error, setError] = useState("");
  const [search, setSearch] = useSearchParams();
  const { resources } = useWorkspace();
  const query = search.get("q") ?? "",
    category = search.get("category") ?? "";
  useEffect(() => {
    let active = true;
    api<Entry[]>("/store/packages")
      .then((r) => {
        if (active) setItems(r);
      })
      .catch((e) => {
        if (active) setError(e.message);
      });
    return () => {
      active = false;
    };
  }, []);
  const filter = (key: string, value: string) => {
    const p = new URLSearchParams(search);
    if (value) p.set(key, value);
    else p.delete(key);
    setSearch(p, { replace: true });
  };
  return (
    <>
      <header className="page-heading">
        <div>
          <div className="eyebrow">PROFILE STORE</div>
          <h1>Profile 商店</h1>
          <p>发现并安装可复用的配置。</p>
        </div>
        <Link className="button" to="/profiles">
          我的 Profiles <Icon name="arrow" size={16} />
        </Link>
      </header>
      <section className="store-intro">
        <Icon name="layers" size={32} />
        <div>
          <h2>安装到配置库，按需关联身份。</h2>
          <p>
            安装只添加到你的配置库。选择身份、指定策略并确认预览后，规则才会生效。
          </p>
        </div>
        <span className="chip">云端编译 · 端侧零扩展</span>
      </section>
      <div className="collection-bar">
        <label className="search-field">
          <Icon name="search" />
          <input
            aria-label="搜索商店"
            placeholder="搜索应用或用途…"
            value={query}
            onChange={(e) => filter("q", e.target.value)}
          />
        </label>
        <select
          aria-label="用途分类"
          value={category}
          onChange={(e) => filter("category", e.target.value)}
        >
          <option value="">全部用途</option>
          {Array.from(new Set(items?.map((p) => p.manifest.category))).map(
            (c) => (
              <option key={c}>{c}</option>
            ),
          )}
        </select>
      </div>
      {error && (
        <p role="alert" className="inline-error">
          {error}
        </p>
      )}
      {!items && !error && <p>正在载入精选目录…</p>}
      {items && (
        <div className="store-grid">
          {items
            .filter(
              (p) =>
                (!category || p.manifest.category === category) &&
                `${p.manifest.name} ${p.manifest.summary}`
                  .toLowerCase()
                  .includes(query.toLowerCase()),
            )
            .map((p) => (
              <Link className="store-card" key={p.slug} to={`/store/${p.slug}`}>
                <div className="store-card-top">
                  <span className="store-symbol">
                    <Icon
                      name={
                        p.manifest.default_policy === "DIRECT"
                          ? "route"
                          : "shield"
                      }
                      size={24}
                    />
                  </span>
                  <span className="chip">{p.manifest.category}</span>
                </div>
                <h2>{p.manifest.name}</h2>
                <p>{p.manifest.summary}</p>
                <div className="store-card-meta">
                  <span>
                    {p.publisher} · v{p.version}
                  </span>
                  <span>
                    {resources.some((r) => r.data.store?.slug === p.slug)
                      ? "已安装"
                      : (p.manifest.default_policy ?? "选择策略组")}{" "}
                    <Icon name="arrow" size={14} />
                  </span>
                </div>
              </Link>
            ))}
        </div>
      )}
      {items?.length === 0 && (
        <section className="panel">
          <h2>精选目录正在准备中</h2>
          <p>这里只展示已经发布的版本，未审核的内容不会自动出现。</p>
        </section>
      )}
      <p className="footnote">
        规则只决定流量去向，不保证节点解锁、服务可用性或网络提速。每个版本保留来源与许可。
      </p>
    </>
  );
}
export function StoreDetail() {
  const { slug } = useParams(),
    navigate = useNavigate();
  const { run, busy, resources } = useWorkspace();
  const [search, setSearch] = useSearchParams();
  const [detail, setDetail] = useState<Detail | null>(null),
    [version, setVersion] = useState(""),
    [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    api<Detail>(`/store/packages/${slug}`)
      .then((d) => {
        if (active) {
          setDetail(d);
          setError("");
          setVersion(d.versions[0]?.id ?? "");
        }
      })
      .catch((e) => {
        if (active) setError(e.message);
      });
    return () => {
      active = false;
    };
  }, [slug]);
  const [installationId, setInstallationId] = useState(() =>
    crypto.randomUUID(),
  );
  const selected =
    detail?.versions.find((v) => v.id === (search.get("version") || version)) ??
    detail?.versions[0];
  const tabs = [
    { id: "overview", label: "概览" },
    { id: "rules", label: "规则预览" },
    { id: "sources", label: "来源与许可" },
  ];
  const tab = tabs.some((t) => t.id === search.get("tab"))
    ? search.get("tab")!
    : "overview";
  const updateSearch = (key: string, value: string) => {
    const next = new URLSearchParams(search);
    next.set(key, value);
    setSearch(next);
  };
  const installed = resources.filter((r) => r.data.store?.slug === slug);
  if (error) return <p role="alert">{error}</p>;
  if (!detail || detail.slug !== slug || !selected) return <p>正在加载组件…</p>;
  return (
    <>
      <Link className="back-link" to="/store">
        <Icon name="back" size={16} /> 全部组件
      </Link>
      <header className="page-heading">
        <div>
          <div className="eyebrow">PROFILE STORE</div>
          <h1>{selected.manifest.name}</h1>
          <p>{selected.manifest.summary}</p>
          <div className="package-byline">
            <span className="chip">{selected.manifest.category}</span>
            <span>{detail.publisher} 发布</span>
            <span>v{selected.version}</span>
          </div>
        </div>
        <button
          className="primary"
          disabled={busy}
          onClick={() =>
            void run(
              () =>
                api<Resource>("/store/install", "POST", {
                  version_id: selected.id,
                  profile_id: installationId,
                }),
              "已添加到配置库，尚未关联任何身份。",
            ).then((r) => {
              if (r) navigate(`/profiles/${r.id}`);
            })
          }
        >
          <Icon name="plus" size={16} />{" "}
          {installed.length ? "再添加一份" : "添加到我的 Profiles"}
        </button>
      </header>
      <div className="tabs" role="tablist" aria-label="组件详情">
        {tabs.map((t) => (
          <button
            key={t.id}
            id={`package-tab-${t.id}`}
            role="tab"
            aria-selected={tab === t.id}
            aria-controls="package-tabpanel"
            className={tab === t.id ? "selected" : ""}
            onClick={() => updateSearch("tab", t.id)}
          >
            {t.label}
            {t.id === "rules" && (
              <span className="package-tab-count">{selected.rules.length}</span>
            )}
          </button>
        ))}
      </div>
      <div className="detail-columns package-detail">
        <div
          id="package-tabpanel"
          role="tabpanel"
          aria-labelledby={`package-tab-${tab}`}
          className="detail-main"
        >
          {tab === "overview" && (
            <>
              <Panel
                title="适用范围"
                actions={<span className="chip">仅修改 rules</span>}
              >
                <PanelBody>
                  <p className="package-description">
                    {selected.manifest.notes}
                  </p>
                  <div className="package-note">
                    <Icon name="shield" size={18} />
                    <p>
                      不修改 DNS、TUN
                      或监听端口。规则直接合入订阅，设备无需安装插件或额外下载规则。
                    </p>
                  </div>
                </PanelBody>
              </Panel>
              <Panel
                title="如何生效"
                actions={<span className="muted">安装不会改变现有网络</span>}
              >
                <ol className="package-steps">
                  <li>
                    <span>01</span>
                    <div>
                      <h3>添加到配置库</h3>
                      <p>
                        安装 v{selected.version}
                        ，内容保持固定。后续由你决定是否升级。
                      </p>
                    </div>
                  </li>
                  <li>
                    <span>02</span>
                    <div>
                      <h3>在身份中启用</h3>
                      <p>
                        {selected.manifest.default_policy
                          ? `默认使用 ${selected.manifest.default_policy}；可在每个身份内分别设置出口和顺序。`
                          : "选择已有策略组或节点作为出口；不同身份可以使用不同策略。"}
                      </p>
                    </div>
                  </li>
                  <li>
                    <span>03</span>
                    <div>
                      <h3>预览后发布</h3>
                      <p>
                        确认最终规则与冲突提示。系统云端直连保护始终最先匹配。
                      </p>
                    </div>
                  </li>
                </ol>
              </Panel>
            </>
          )}
          {tab === "rules" && (
            <ConfigPreview
              title="合并规则"
              description="置于原有规则之前，实际出口以身份中的选择为准。"
              actions={<span className="chip">{selected.rules.length} 条</span>}
              content={selected.rules
                .map(
                  (r) =>
                    `${r.kind},${r.value},${selected.manifest.default_policy ?? "<身份策略>"}${r.no_resolve ? ",no-resolve" : ""}`,
                )
                .join("\n")}
            />
          )}
          {tab === "sources" && (
            <Panel
              title="来源与许可"
              description="每个版本固定来源提交与内容哈希，保留作者署名。"
              actions={
                <span className="chip">
                  {selected.manifest.sources.length} 个文件
                </span>
              }
            >
              <div className="package-sources">
                {selected.manifest.sources.map((s) => (
                  <article className="package-source" key={s.url}>
                    <div className="package-source-heading">
                      <a
                        className="text-link"
                        href={s.url}
                        target="_blank"
                        rel="noreferrer"
                      >
                        {decodeURIComponent(
                          new URL(s.url).pathname
                            .split("/")
                            .slice(-2)
                            .join("/"),
                        )}
                        <Icon name="arrow" size={14} />
                      </a>
                      <span className="chip">{s.license}</span>
                    </div>
                    <p>{s.attribution}</p>
                    <details className="package-disclosure">
                      <summary>来源校验信息</summary>
                      <dl className="package-checks">
                        <dt>Commit</dt>
                        <dd>
                          <code>{s.revision}</code>
                        </dd>
                        <dt>SHA256</dt>
                        <dd>
                          <code>{s.sha256}</code>
                        </dd>
                      </dl>
                    </details>
                    <details className="package-disclosure">
                      <summary>许可全文</summary>
                      <pre>{s.license_text}</pre>
                    </details>
                  </article>
                ))}
              </div>
            </Panel>
          )}
        </div>
        <aside className="package-aside">
          <Panel title="组件信息" actions={<Icon name="layers" size={17} />}>
            <PanelBody>
              <label className="package-field">
                选择版本
                <select
                  value={selected.id}
                  onChange={(e) => {
                    updateSearch("version", e.target.value);
                    setInstallationId(crypto.randomUUID());
                  }}
                >
                  {detail.versions.map((v) => (
                    <option value={v.id} key={v.id}>
                      v{v.version}
                    </option>
                  ))}
                </select>
              </label>
              <dl className="package-metadata">
                <div>
                  <dt>默认策略</dt>
                  <dd>{selected.manifest.default_policy ?? "按身份指定"}</dd>
                </div>
                <div>
                  <dt>规则数量</dt>
                  <dd>{selected.rules.length} 条</dd>
                </div>
                <div>
                  <dt>更新方式</dt>
                  <dd>手动升级</dd>
                </div>
              </dl>
              {installed.length > 0 && (
                <Link className="text-link" to={`/profiles/${installed[0].id}`}>
                  查看已安装的 Profile <Icon name="arrow" size={14} />
                </Link>
              )}
              <details className="package-disclosure">
                <summary>版本校验信息</summary>
                <code className="store-hash">{selected.hash}</code>
                <Copy value={selected.hash} label="复制 SHA256" />
              </details>
            </PanelBody>
          </Panel>
          <Panel title="客户端支持">
            <PanelBody className="package-compatibility">
              <div>
                <strong>Mihomo / Clash Verge</strong>
                <p>支持此组件使用的基础规则。</p>
              </div>
              <div>
                <strong>Shadowrocket</strong>
                <p>可导出基础规则；完整身份仍需兼容校验。</p>
              </div>
              <p className="package-caption">仅节点订阅不包含分流规则。</p>
            </PanelBody>
          </Panel>
        </aside>
      </div>
    </>
  );
}
export function ManagedSource({ resource: r }: { resource: Resource }) {
  const [policy, setPolicy] = useState("");
  const [result, setResult] = useState<{
    content: string;
    policy: string;
    parameterized: boolean;
  } | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    const timer = setTimeout(
      () => {
        api<{ content: string; policy: string; parameterized: boolean }>(
          `/profiles/${r.id}/source-preview`,
          "POST",
          { version: r.version, policy: policy || null },
        )
          .then((v) => {
            if (active) setResult(v);
          })
          .catch((e) => {
            if (active) setError(e.message);
          });
      },
      policy ? 250 : 0,
    );
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [r.id, r.version, policy]);
  return (
    <ConfigPreview
      title="YAML 源码"
      description={`已安装 v${r.data._package?.version} · 云端编译 · 只读`}
      actions={
        <Link className="text-link" to={`/store/${r.data.store?.slug}`}>
          组件详情 <Icon name="arrow" size={14} />
        </Link>
      }
      controls={
        <div className="package-source-controls">
          <label className="package-field">
            预览策略
            <input
              aria-label="YAML 预览策略"
              value={policy}
              onChange={(e) => {
                setPolicy(e.target.value);
                setResult(null);
                setError("");
              }}
              placeholder={
                r.data._package?.manifest.default_policy ??
                "留空显示 <身份策略> 占位符"
              }
              maxLength={200}
            />
          </label>
          <p>
            仅改变源码预览，不修改任何身份的出口。最终配置请到身份的「合并预览」查看。
          </p>
        </div>
      }
      error={error}
      loading={!result && !error}
      content={result?.content}
      warnings={
        result?.parameterized
          ? [
              "此组件需要身份策略；<身份策略> 是占位符，不能直接作为完整订阅使用。",
            ]
          : []
      }
    />
  );
}
export function ManagedProfile({ resource: r }: { resource: Resource }) {
  const { run, busy } = useWorkspace(),
    navigate = useNavigate();
  const [detail, setDetail] = useState<Detail | null>(null),
    [error, setError] = useState(""),
    [target, setTarget] = useState(r.data.store?.version_id ?? ""),
    [preview, setPreview] = useState<Upgrade | null>(null),
    [policy, setPolicy] = useState(
      r.data._package?.manifest.default_policy ?? "",
    );
  useEffect(() => {
    let active = true;
    api<Detail>(`/store/packages/${r.data.store?.slug}`)
      .then((d) => {
        if (active) setDetail(d);
      })
      .catch((e) => {
        if (active) setError(e.message);
      });
    return () => {
      active = false;
    };
  }, [r.data.store?.slug]);
  return (
    <div className="detail-main">
      <Panel
        title="版本管理"
        actions={<span className="chip">v{r.data._package?.version}</span>}
      >
        <PanelBody>
          <p className="package-description">
            当前版本保持锁定，不会随上游自动变化。切换版本前，会一起验证所有启用此
            Profile 的身份。
          </p>
          {error && (
            <p className="inline-error" role="alert">
              {error}
            </p>
          )}
          <FieldActionRow>
            <label className="package-field">
              目标版本
              <select
                value={target}
                onChange={(e) => {
                  setTarget(e.target.value);
                  setPreview(null);
                }}
              >
                {(detail?.versions ?? []).map((v) => (
                  <option key={v.id} value={v.id}>
                    {v.version}
                    {v.id === r.data.store?.version_id ? "（当前）" : ""}
                  </option>
                ))}
              </select>
            </label>
            <button
              disabled={busy || !detail || target === r.data.store?.version_id}
              onClick={() =>
                void run(() =>
                  api<Upgrade>(`/profiles/${r.id}/upgrade-preview`, "POST", {
                    version: r.version,
                    version_id: target,
                  }),
                ).then((p) => {
                  if (p) setPreview(p);
                })
              }
            >
              预览版本变更
            </button>
          </FieldActionRow>
          {detail?.versions.length === 1 && (
            <p className="package-caption">
              当前只有一个已发布版本，暂无其他版本可切换。
            </p>
          )}
          {preview && (
            <div className="store-change">
              <h3>
                新增 {preview.added} 条 · 删除 {preview.removed} 条
              </h3>
              <p>
                影响 {preview.affected.length}{" "}
                个已启用身份。更新后保持手动锁定。
              </p>
              {preview.affected.map((b) => (
                <div key={b.id}>
                  <strong>{b.name}</strong>
                  {b.warnings.map((w, i) => (
                    <p className="inline-error" key={i}>
                      {w}
                    </p>
                  ))}
                  {Object.entries(b.outputs)
                    .filter(([, v]) => v.error)
                    .map(([f, v]) => (
                      <p key={f} className="inline-error">
                        {f}：{v.error}
                      </p>
                    ))}
                </div>
              ))}
              <button
                className="primary"
                disabled={busy}
                onClick={() =>
                  void run(
                    () =>
                      api(`/profiles/${r.id}/upgrade`, "POST", {
                        version: r.version,
                        version_id: target,
                        preview_digest: preview.preview_digest,
                      }),
                    "版本已更新并锁定。",
                  ).then(() => setPreview(null))
                }
              >
                确认应用该版本
              </button>
            </div>
          )}
        </PanelBody>
      </Panel>
      <Panel title="创建独立副本" actions={<Icon name="copy" size={17} />}>
        <PanelBody>
          <p className="package-description">
            复制为独立 Profile 后可以自由编辑
            YAML，并保留来源说明。已有身份仍使用原组件，不会自动切换。
          </p>
          <FieldActionRow>
            <label className="package-field">
              副本采用的策略
              <input
                value={policy}
                onChange={(e) => setPolicy(e.target.value)}
                placeholder="DIRECT / REJECT / 身份中的策略组名称"
              />
            </label>
            <button
              disabled={busy || !policy}
              onClick={() =>
                void run(
                  () =>
                    api<Resource>(`/profiles/${r.id}/fork`, "POST", {
                      version: r.version,
                      policy,
                    }),
                  "已创建独立副本。",
                ).then((x) => {
                  if (x) navigate(`/profiles/${x.id}`);
                })
              }
            >
              <Icon name="copy" size={16} />
              复制为独立 Profile
            </button>
          </FieldActionRow>
        </PanelBody>
      </Panel>
    </div>
  );
}
export function AccountPage({
  user,
  onChange,
}: {
  user: User;
  onChange: (u: User) => void;
}) {
  const { busy, run } = useWorkspace();
  const [name, setName] = useState(user.nickname ?? "");
  return (
    <>
      <header className="page-heading">
        <div>
          <div className="eyebrow">YOUR ACCOUNT</div>
          <h1>个人资料</h1>
          <p>选择对外展示的名字。登录邮箱不会出现在商店商品上。</p>
        </div>
      </header>
      <section className="panel account-panel">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void run(
              () => api<User>("/account", "PATCH", { nickname: name }),
              "昵称已更新。",
            ).then((u) => {
              if (u) onChange(u);
            });
          }}
        >
          <label>
            昵称
            <input
              required
              maxLength={40}
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          <label>
            登录邮箱
            <input value={user.email} readOnly aria-readonly="true" />
          </label>
          <p className="muted">
            邮箱用于登录，不能在此修改。昵称会同步到你发布的商店组件。
          </p>
          <button className="primary" disabled={busy}>
            保存个人资料
          </button>
        </form>
      </section>
    </>
  );
}
