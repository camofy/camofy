import { useState, useRef, useCallback } from "react";
import { useBlocker, useBeforeUnload } from "react-router-dom";
import { api, type Data, type Resource, type User } from "./model";
import { Modal } from "./ui";

export function Login({ onLogin }: { onLogin: (u: User) => void }) {
  const [register, setRegister] = useState(false),
    [email, setEmail] = useState(""),
    [password, setPassword] = useState(""),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false);
  return (
    <div className="login">
      <section>
        <span className="brand">
          camofy<span>cloud</span>
        </span>
        <h1>一份配置，连接每一端。</h1>
        <p className="muted">
          管理订阅与功能配置，让你的设备使用一致的网络策略。
        </p>
        <form
          onSubmit={async (e) => {
            e.preventDefault();
            setBusy(true);
            setError("");
            try {
              onLogin(
                await api<User>(
                  `/auth/${register ? "register" : "login"}`,
                  "POST",
                  { email, password },
                ),
              );
            } catch (e) {
              setError(e instanceof Error ? e.message : String(e));
            } finally {
              setBusy(false);
            }
          }}
        >
          <label>
            邮箱
            <input
              type="email"
              autoComplete="email"
              required
              value={email}
              onChange={(e) => setEmail(e.target.value)}
            />
          </label>
          <label>
            密码
            <input
              type="password"
              minLength={12}
              maxLength={256}
              autoComplete={register ? "new-password" : "current-password"}
              required
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </label>
          <p className="muted">至少 12 个字符。</p>
          {error && (
            <p role="alert" className="inline-error">
              {error}
            </p>
          )}
          <button className="primary" disabled={busy}>
            {busy ? "请稍候…" : register ? "创建账号" : "登录"}
          </button>
        </form>
        <button onClick={() => setRegister(!register)}>
          {register ? "已有账号？登录" : "首次使用？注册账号"}
        </button>
        <p className="footnote">
          此实例可独立自托管，账号与数据留在当前服务器。
        </p>
      </section>
    </div>
  );
}
export function Editor({
  resource,
  all,
  busy,
  serverError,
  onSave,
  onSaved,
  onClose,
}: {
  resource: Resource;
  all: Resource[];
  busy: boolean;
  serverError: string;
  onSave: (r: Resource) => Promise<Resource | undefined>;
  onSaved: (r: Resource) => void;
  onClose: () => void;
}) {
  const [original] = useState(resource);
  const [data, setData] = useState<Data>(structuredClone(resource.data)),
    [selections, setSelections] = useState(
      JSON.stringify(resource.data.selections ?? {}, null, 2),
    ),
    [error, setError] = useState("");
  const [baseline] = useState(() => JSON.stringify(resource.data));
  const [initialSelections] = useState(selections);
  const saved = useRef(false);
  const dirty =
    JSON.stringify(data) !== baseline || selections !== initialSelections;
  const blocker = useBlocker(() => dirty && !saved.current);
  useBeforeUnload(
    useCallback(
      (e: BeforeUnloadEvent) => {
        if (dirty) e.preventDefault();
      },
      [dirty],
    ),
  );
  const set = (key: keyof Data, value: unknown) =>
    setData((d) => ({ ...d, [key]: value }));
  const [egress, setEgress] = useState<{
    ip: string;
    proof: unknown;
    expires_at: number;
  } | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const options = (kind: string, type?: string) =>
    all.filter((r) => r.kind === kind && (!type || r.data.type === type));
  const reorder = (i: number, delta: number) => {
    const ids = [...(data.profiles ?? [])];
    [ids[i], ids[i + delta]] = [ids[i + delta], ids[i]];
    set("profiles", ids);
  };
  return (
    <section className="editor-panel">
      {blocker.state === "blocked" && (
        <Modal title="放弃未保存的修改？" close={() => blocker.reset()}>
          <p>离开后，本页未保存的修改将丢失。</p>
          <div className="form-actions">
            <button onClick={() => blocker.reset()}>继续编辑</button>
            <button className="danger" onClick={() => blocker.proceed()}>
              放弃修改
            </button>
          </div>
        </Modal>
      )}
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          setError("");
          try {
            const next = { ...data };
            if (resource.kind === "bundle")
              next.selections = JSON.parse(selections);
            const result = await onSave({ ...original, data: next });
            if (result) {
              saved.current = true;
              onSaved(result);
            }
          } catch {
            setError("节点选择必须是有效的 JSON 对象。");
          }
        }}
      >
        <label>
          名称
          <input
            required
            maxLength={120}
            value={data.name}
            onChange={(e) => set("name", e.target.value)}
          />
        </label>
        {resource.kind === "proxy" && (
          <>
            <label>
              供应商
              <select
                value={data.provider ?? "static"}
                onChange={(e) => set("provider", e.target.value)}
              >
                <option value="static">固定代理</option>
                <option value="xiequ">携趣 · 短效代理</option>
              </select>
            </label>
            {data.provider === "xiequ" ? (
              <>
                <label>
                  提取代理 IP 的完整接口链接
                  <input
                    type="password"
                    autoComplete="new-password"
                    required={
                      !resource.id || resource.data.provider !== "xiequ"
                    }
                    value={data.extract_url ?? ""}
                    placeholder="http://api.xiequ.cn/VAD/GetIp.aspx?...&num=1..."
                    onChange={(e) => set("extract_url", e.target.value)}
                  />
                </label>
                <label>
                  白名单账号 uid
                  <input
                    required
                    value={data.whitelist_uid ?? ""}
                    onChange={(e) => set("whitelist_uid", e.target.value)}
                  />
                </label>
                <label>
                  白名单密钥 ukey
                  <input
                    type="password"
                    autoComplete="new-password"
                    required={
                      !resource.id || resource.data.provider !== "xiequ"
                    }
                    value={data.whitelist_key ?? ""}
                    onChange={(e) => set("whitelist_key", e.target.value)}
                  />
                </label>
                <label>
                  提取的代理协议
                  <select
                    value={data.protocol ?? "http"}
                    onChange={(e) => set("protocol", e.target.value)}
                  >
                    <option value="http">HTTP / HTTPS（HTTP CONNECT）</option>
                    <option value="socks5">SOCKS5</option>
                  </select>
                </label>
                <p className="muted">
                  每次刷新即时提取 1 个
                  IP，不缓存、不自动重试扣费请求、不回退直连。密钥留空保留原值。HTTP
                  提取链接会明文传输密钥；白名单管理使用 HTTPS。
                </p>
                <p>已确认的白名单 IP：{data.whitelist_ip ?? "尚未配置"}</p>
                <button
                  type="button"
                  disabled={previewing || busy}
                  onClick={() => {
                    setPreviewing(true);
                    setError("");
                    setEgress(null);
                    set("egress_preview", undefined);
                    void api<{
                      ip: string;
                      proof: unknown;
                      expires_at: number;
                    }>("/proxies/egress-preview", "POST", {})
                      .then(setEgress)
                      .catch((e) => setError(e.message))
                      .finally(() => setPreviewing(false));
                  }}
                >
                  {previewing ? "正在从服务器核对出口…" : "预览服务器公网 IPv4"}
                </button>
                {egress && (
                  <label className="check">
                    <input
                      type="checkbox"
                      checked={!!data.egress_preview}
                      onChange={(e) =>
                        set(
                          "egress_preview",
                          e.target.checked ? egress.proof : undefined,
                        )
                      }
                    />
                    确认将服务器 {egress.ip} 加入携趣白名单（预览 5 分钟有效）
                  </label>
                )}
                <p className="muted">
                  新建或更改接口/凭据时需预览并确认。保存时再次核对出口，只添加这一条，不删除已有白名单。
                </p>
              </>
            ) : (
              <>
                <label>
                  代理 URL
                  <input
                    required={!resource.id}
                    type="text"
                    placeholder="socks5://user:password@proxy.example.com:1080"
                    value={data.url ?? ""}
                    onChange={(e) => set("url", e.target.value)}
                  />
                </label>
                <p className="muted">
                  支持 SOCKS5、HTTP 和 HTTPS。已有代理留空会保留原凭据。
                </p>
              </>
            )}
          </>
        )}
        {resource.kind === "profile" && data.type === "source" && (
          <>
            <label>
              Clash YAML 订阅 URL
              <input
                required
                type="url"
                value={data.url ?? ""}
                onChange={(e) => set("url", e.target.value)}
              />
            </label>
            <label>
              拉取代理
              <select
                value={data.proxy_id ?? ""}
                onChange={(e) => set("proxy_id", e.target.value || null)}
              >
                <option value="">直连</option>
                {options("proxy").map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.data.type === "source"
                      ? "订阅源 · "
                      : r.data.type === "overlay"
                        ? "配置 · "
                        : ""}
                    {r.data.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={data.auto_refresh !== false}
                onChange={(e) => set("auto_refresh", e.target.checked)}
              />
              定时刷新
            </label>
            <label>
              刷新间隔（分钟）
              <input
                type="number"
                min={5}
                max={10080}
                required
                value={(data.interval_seconds ?? 3600) / 60}
                onChange={(e) =>
                  set("interval_seconds", Number(e.target.value) * 60)
                }
              />
            </label>
            <p className="muted">
              定时刷新与立即刷新均通过上面选择的代理。失败会保留上次成功内容。
            </p>
            <label>
              共用额度池（可选）
              <input value={data.usage_pool ?? ""} maxLength={120} placeholder="例如：我的主套餐" onChange={(e) => set("usage_pool", e.target.value)} />
            </label>
            <p className="muted">同一套餐的不同链接填写相同名称，身份统计只计一次。留空时按相同上游 URL 自动去重，不按机场域名猜测。</p>
          </>
        )}
        {resource.kind === "profile" && data.type === "overlay" && (
          <>
            <label>
              独立配置 YAML
              <textarea
                className="code"
                rows={16}
                required
                value={data.content ?? ""}
                onChange={(e) => set("content", e.target.value)}
              />
            </label>
            <p className="muted">
              节点/代理组按名称合并，后者覆盖同名项；规则按顺序拼接；对象深合并，其他数组覆盖。
              支持 prepend-/append-rules、proxies、proxy-groups；prepend
              将内容放到已有列表前面。
            </p>
          </>
        )}
        {resource.kind === "bundle" && (
          <>
            <label>
              关联的 Profile（从上到下合并，可混合多个订阅和独立配置）
            </label>
            {(data.profiles ?? []).map((binding, i) => (
              <div key={binding.profile_id} className="overlay-row">
                <label className="check">
                  <input
                    type="checkbox"
                    checked={binding.enabled}
                    onChange={(e) =>
                      set(
                        "profiles",
                        data.profiles?.map((x, index) =>
                          index === i ? { ...x, enabled: e.target.checked } : x,
                        ),
                      )
                    }
                  />
                  {all.find((r) => r.id === binding.profile_id)?.data.name}
                </label>
                <button
                  type="button"
                  disabled={i === 0}
                  onClick={() => reorder(i, -1)}
                >
                  上移
                </button>
                <button
                  type="button"
                  disabled={i === (data.profiles?.length ?? 0) - 1}
                  onClick={() => reorder(i, 1)}
                >
                  下移
                </button>
                <button
                  type="button"
                  onClick={() =>
                    set(
                      "profiles",
                      data.profiles?.filter(
                        (x) => x.profile_id !== binding.profile_id,
                      ),
                    )
                  }
                >
                  移除关联
                </button>
              </div>
            ))}
            <select
              aria-label="关联 Profile"
              value=""
              onChange={(e) => {
                if (e.target.value)
                  set("profiles", [
                    ...(data.profiles ?? []),
                    { profile_id: e.target.value, enabled: true },
                  ]);
              }}
            >
              <option value="">选择订阅源或配置 Profile…</option>
              {options("profile")
                .filter(
                  (r) =>
                    !data.profiles?.some(
                      (binding) => binding.profile_id === r.id,
                    ),
                )
                .map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.data.type === "source" ? "订阅源 · " : "配置 · "}
                    {r.data.name}
                  </option>
                ))}
            </select>
            <label>
              共享节点选择（代理组 → 节点名称）
              <textarea
                className="code"
                rows={5}
                value={selections}
                onChange={(e) => setSelections(e.target.value)}
              />
            </label>
            <p className="muted">
              例如 {`{"服务节点":"我的节点"}`}。Agent
              会立即应用；第三方客户端将收到首选节点顺序，已有本地选择可能优先。
            </p>
          </>
        )}
        {resource.kind === "device" && (
          <>
            <label>
              身份
              <select
                required
                value={data.bundle_id ?? ""}
                onChange={(e) => set("bundle_id", e.target.value)}
              >
                <option value="">请选择</option>
                {options("bundle").map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.data.name}
                  </option>
                ))}
              </select>
            </label>
            <p className="muted">
              保存后自动向设备下发这个身份，无需到路由器配置订阅
              URL。设备离线时会在重连后同步。
            </p>
          </>
        )}
        {(error || serverError) && (
          <p role="alert" className="inline-error">
            {error || serverError}
          </p>
        )}
        <div className="form-actions">
          <button type="button" onClick={onClose}>
            取消
          </button>
          <button className="primary" disabled={busy}>
            {busy ? "正在保存…" : "保存"}
          </button>
        </div>
      </form>
    </section>
  );
}
