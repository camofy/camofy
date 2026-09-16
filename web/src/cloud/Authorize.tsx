import { useEffect, useState } from "react";
import brandMark from "../assets/flower.png";
import { useSearchParams, Link } from "react-router-dom";
import { api, type Resource, type User } from "./model";
import { Icon } from "./ui";

type Request = { device_name: string; return_uri: string; scope: string };
export default function Authorize({
  user,
  logout,
}: {
  user: User;
  logout: () => void;
}) {
  const [params, setParams] = useSearchParams();
  const code = params.get("user_code") ?? "";
  const [manual, setManual] = useState("");
  const [loaded, setLoaded] = useState<{
    code: string;
    request?: Request;
    identities?: Resource[];
    error?: string;
  } | null>(null);
  const [identity, setIdentity] = useState("");
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  useEffect(() => {
    if (!code) return;
    let active = true;
    void Promise.all([
      api<Request>(`/oauth/requests/${encodeURIComponent(code)}`),
      api<Resource[]>("/resources"),
    ])
      .then(([request, resources]) => {
        if (active)
          setLoaded({
            code,
            request,
            identities: resources.filter((r) => r.kind === "bundle"),
          });
      })
      .catch((e) => {
        if (active) setLoaded({ code, error: e.message });
      });
    return () => {
      active = false;
    };
  }, [code]);
  const current = loaded?.code === code ? loaded : null;
  async function respond(approve: boolean) {
    setBusy(true);
    setError("");
    try {
      const result = await api<{ return_uri: string }>(
        "/oauth/approve",
        "POST",
        { user_code: code, approve, bundle_id: approve ? identity : null },
      );
      const target = new URL(result.return_uri);
      const expected = new URL(current!.request!.return_uri);
      if (
        target.origin !== expected.origin ||
        target.pathname !== expected.pathname
      )
        throw Error("返回地址校验失败，请手动返回路由器。");
      window.location.assign(target.href);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  }
  return (
    <main className="authorize-wrap">
      <section className="authorize-card">
        <Link className="authorize-brand" to="/identities">
          <img className="brand-symbol" src={brandMark} alt="" />
          camofy <small>CLOUD</small>
        </Link>
        <div className="authorize-account">
          <span>当前账号</span>
          <strong>{user.email}</strong>
          <button className="quiet" disabled={busy} onClick={logout}>
            切换账号
          </button>
        </div>
        <span className="resource-icon">
          <Icon name="monitor" size={24} />
        </span>
        <h1>连接你的设备</h1>
        <p className="muted">
          确认这是你刚刚从设备页面发起的请求。授权后，该设备只能同步所选身份，不能管理你的账号。
        </p>
        {!code ? (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              setParams({ user_code: manual.trim() });
            }}
          >
            <label>
              设备页面上的授权码
              <input
                required
                value={manual}
                onChange={(e) => setManual(e.target.value)}
                autoComplete="off"
                placeholder="XXXXXXXX-XXXXXXXX"
              />
            </label>
            <button className="primary full-width">继续</button>
          </form>
        ) : !current ? (
          <p className="panel-message" role="status">
            正在检查授权请求…
          </p>
        ) : current.error ? (
          <div className="inline-error" role="alert">
            {current.error}
          </div>
        ) : (
          <>
            <div className="authorize-device">
              <strong>{current.request!.device_name}</strong>
              <span>{new URL(current.request!.return_uri).origin}</span>
              <code>{code}</code>
            </div>
            <p className="authorization-warning">
              <Icon name="shield" size={18} />
              请核对设备名称、地址及授权码。不要批准他人通过聊天或邮件发来的授权请求。
            </p>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                void respond(true);
              }}
            >
              <label>
                这台设备使用哪个身份？
                <select
                  required
                  value={identity}
                  onChange={(e) => setIdentity(e.target.value)}
                >
                  <option value="">请选择身份</option>
                  {current.identities!.map((r) => (
                    <option
                      key={r.id}
                      value={r.id}
                      disabled={!r.data.published_revision}
                    >
                      {r.data.name}
                      {!r.data.published_revision ? "（尚未成功发布）" : ""}
                    </option>
                  ))}
                </select>
              </label>
              {!current.identities!.some((r) => r.data.published_revision) && (
                <p className="muted">
                  你还没有可用身份。
                  <Link
                    className="text-link"
                    target="_blank"
                    rel="noopener noreferrer"
                    to="/identities/new"
                  >
                    先创建身份
                  </Link>
                  ，然后刷新本页。
                </p>
              )}
              <p className="muted">
                设备将按所选身份的配置运行，包括其中的 TUN
                设置。请确认配置适合当前设备；可随时在云端撤销设备凭据。
              </p>
              {error && (
                <p className="inline-error" role="alert">
                  {error}
                </p>
              )}
              <button
                className="primary full-width"
                disabled={busy || !identity}
              >
                {busy ? "正在处理…" : "授权并返回设备"}
              </button>
              <button
                type="button"
                className="full-width"
                disabled={busy}
                onClick={() => {
                  void respond(false);
                }}
              >
                取消授权并返回
              </button>
            </form>
          </>
        )}
        <p className="footnote">
          账号密码只在此云端登录页面使用。路由器网页和回跳链接不包含长期访问凭据。
        </p>
      </section>
    </main>
  );
}
