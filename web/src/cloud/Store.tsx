import { useEffect, useState } from "react";
import {
  Link,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router-dom";
import { api, type Resource, type User } from "./model";
import { useWorkspace } from "./context";
import { CodeBlock, Icon } from "./ui";
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
          <h1>为你的网络，添加一种能力。</h1>
          <p>精选规则 · 固定版本 · 自由组合</p>
        </div>
        <Link className="button" to="/profiles">
          我的 Profiles <Icon name="arrow" size={16} />
        </Link>
      </header>
      <section className="store-intro">
        <Icon name="layers" size={32} />
        <div>
          <h2>小组件，组成你的网络习惯。</h2>
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
  const { run, busy } = useWorkspace();
  const [detail, setDetail] = useState<Detail | null>(null),
    [version, setVersion] = useState(""),
    [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    api<Detail>(`/store/packages/${slug}`)
      .then((d) => {
        if (active) {
          setDetail(d);
          setVersion(d.versions[0].id);
        }
      })
      .catch((e) => {
        if (active) setError(e.message);
      });
    return () => {
      active = false;
    };
  }, [slug]);
  const [installationId] = useState(() => crypto.randomUUID());
  const selected = detail?.versions.find((v) => v.id === version);
  if (error) return <p role="alert">{error}</p>;
  if (!detail || !selected) return <p>正在加载组件…</p>;
  return (
    <>
      <Link className="back-link" to="/store">
        ← 全部组件
      </Link>
      <header className="page-heading">
        <div>
          <div className="eyebrow">
            {detail.publisher} / {detail.slug}
          </div>
          <h1>{selected.manifest.name}</h1>
          <p>{selected.manifest.summary}</p>
        </div>
        <button
          className="primary"
          disabled={busy}
          onClick={() =>
            void run(
              () =>
                api<Resource>("/store/install", "POST", {
                  version_id: version,
                  profile_id: installationId,
                }),
              "已添加到配置库，尚未关联任何身份。",
            ).then((r) => {
              if (r) navigate(`/profiles/${r.id}`);
            })
          }
        >
          添加到我的 Profiles
        </button>
      </header>
      <div className="store-detail-grid">
        <section className="panel store-content">
          <div className="panel-heading">
            <h2>这个组件会做什么</h2>
            <span className="chip">仅修改 rules</span>
          </div>
          <p>{selected.manifest.notes}</p>
          <dl className="store-facts">
            <div>
              <dt>默认策略</dt>
              <dd>
                {selected.manifest.default_policy ?? "关联身份时选择策略组"}
              </dd>
            </div>
            <div>
              <dt>规则数量</dt>
              <dd>{selected.rules.length}</dd>
            </div>
            <div>
              <dt>安装方式</dt>
              <dd>固定版本 · 手动升级</dd>
            </div>
            <div>
              <dt>运行方式</dt>
              <dd>内联规则，无脚本、无额外下载</dd>
            </div>
          </dl>
          <h3>规则预览</h3>
          <p className="muted">
            实际出口由身份关联参数决定；规则放在既有规则之前，系统云端保护仍最先匹配。
          </p>
          <CodeBlock
            content={selected.rules
              .map(
                (r) =>
                  `${r.kind},${r.value},${selected.manifest.default_policy ?? "<身份策略>"}${r.no_resolve ? ",no-resolve" : ""}`,
              )
              .join("\n")}
          />
        </section>
        <aside className="store-aside">
          <section className="panel">
            <h2>版本与兼容性</h2>
            <label>
              选择版本
              <select
                value={version}
                onChange={(e) => setVersion(e.target.value)}
              >
                {detail.versions.map((v) => (
                  <option value={v.id} key={v.id}>
                    {v.version}
                  </option>
                ))}
              </select>
            </label>
            <p>Mihomo：支持所列基础规则。</p>
            <p>Shadowrocket 完整配置：基础规则可导出，完整身份仍需兼容校验。</p>
            <p>仅节点订阅不包含这些规则。</p>
            <small className="muted">SHA256</small>
            <code className="store-hash">{selected.hash}</code>
          </section>
          <section className="panel">
            <h2>来源与许可</h2>
            {selected.manifest.sources.map((s) => (
              <div className="store-source" key={s.url}>
                <a href={s.url} target="_blank" rel="noreferrer">
                  查看上游来源 ↗
                </a>
                <p>{s.attribution}</p>
                <span className="chip">{s.license}</span>
                <code className="store-hash">commit {s.revision}</code>
                <details>
                  <summary>许可全文</summary>
                  <pre>{s.license_text}</pre>
                </details>
              </div>
            ))}
          </section>
        </aside>
      </div>
    </>
  );
}
export function ManagedProfile({ resource: r }: { resource: Resource }) {
  const { run, busy } = useWorkspace(),
    navigate = useNavigate();
  const [detail, setDetail] = useState<Detail | null>(null),
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
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [r.data.store?.slug]);
  return (
    <section className="panel store-content">
      <div className="panel-heading">
        <h2>商店托管组件</h2>
        <span className="chip">v{r.data._package?.version}</span>
      </div>
      <p>{r.data._package?.manifest.summary}</p>
      <Link to={`/store/${r.data.store?.slug}`}>查看商品、来源和许可 →</Link>
      <p className="muted">
        内容由固定版本管理；启用与出口在身份内设置。升级会验证所有启用此 Profile
        的身份。
      </p>
      <label>
        升级或回滚到版本
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
      {preview && (
        <div className="store-change">
          <h3>
            新增 {preview.added} 条 · 删除 {preview.removed} 条
          </h3>
          <p>
            影响 {preview.affected.length} 个已启用身份。更新后保持手动锁定。
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
      <hr />
      <h3>需要自由编辑？</h3>
      <p>
        复制为独立
        Profile，保留来源说明，但不再跟随商店。已有身份不会自动切换到副本。
      </p>
      <label>
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
        复制为独立 Profile
      </button>
    </section>
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
