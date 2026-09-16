import { useCallback, useEffect, useState } from "react";
import brandMark from "./assets/flower.png";
import {
  createBrowserRouter,
  RouterProvider,
  Routes,
  Route,
  Navigate,
  NavLink,
  Link,
  useLocation,
} from "react-router-dom";
import { api, type Resource, type User } from "./cloud/model";
import { WorkspaceContext } from "./cloud/context";
import { Login } from "./cloud/Forms";
import Authorize from "./cloud/Authorize";
import { StorePage, StoreDetail, AccountPage } from "./cloud/Store";
import { Icon } from "./cloud/ui";
import { sections, sectionOf } from "./cloud/navigation";
import { CollectionPage, DetailPage, EditPage, NotFound } from "./cloud/pages";

function CloudWorkspace() {
  const [user, setUser] = useState<User | null>(null);
  const [ready, setReady] = useState(false),
    [loading, setLoading] = useState(true);
  const [resources, setResources] = useState<Resource[]>([]);
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const [connected, setConnected] = useState(false),
    [mobile, setMobile] = useState(false);
  const location = useLocation();
  useEffect(() => {
    if (!mobile) return;
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMobile(false);
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [mobile]);
  const load = useCallback(async () => {
    try {
      const [r, account] = await Promise.all([
        api<Resource[]>("/resources"),
        api<User>("/auth/me"),
      ]);
      setUser((previous) =>
        previous && JSON.stringify(previous) === JSON.stringify(account)
          ? previous
          : account,
      );
      setResources(r);
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => {
    void api<User>("/auth/me")
      .then(setUser)
      .catch(() => {})
      .finally(() => setReady(true));
  }, []);
  useEffect(() => {
    if (!user) return;
    let closed = false,
      socket: WebSocket,
      reconnect: ReturnType<typeof setTimeout>;
    const refresh = () => {
      void load().catch((e) => {
        if (!closed) setError(e.message);
      });
    };
    const connect = () => {
      if (import.meta.env.DEV && import.meta.env.MODE === "design") return;
      socket = new WebSocket(
        `${window.location.protocol === "https:" ? "wss" : "ws"}://${window.location.host}/api/sync/ws`,
      );
      socket.onopen = () => {
        if (!closed) setConnected(true);
      };
      socket.onmessage = refresh;
      socket.onclose = () => {
        if (!closed) {
          setConnected(false);
          reconnect = setTimeout(connect, 5000);
        }
      };
    };
    connect();
    refresh();
    const timer = setInterval(refresh, 15000);
    return () => {
      closed = true;
      clearTimeout(reconnect);
      clearInterval(timer);
      socket?.close();
    };
  }, [user, load]);
  useEffect(() => {
    if (!notice) return;
    const t = setTimeout(() => setNotice(""), 4500);
    return () => clearTimeout(t);
  }, [notice]);
  async function run<T>(
    fn: () => Promise<T>,
    message?: string,
  ): Promise<T | undefined> {
    setBusy(true);
    setError("");
    try {
      const value = await fn();
      await load();
      if (message) setNotice(message);
      return value;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return undefined;
    } finally {
      setBusy(false);
    }
  }
  const save = (r: Resource) =>
    run(
      () =>
        api<Resource>(
          r.kind === "proxy"
            ? `/admin/proxies${r.id ? `/${r.id}` : ""}`
            : r.id
              ? `/resources/${r.id}`
              : "/resources",
          r.id ? "PUT" : "POST",
          {
            ...(r.kind === "proxy" ? {} : { kind: r.kind }),
            version: r.id ? r.version : undefined,
            data: r.data,
          },
        ),
      r.kind === "proxy" ? "平台代理已保存。" : "修改已保存。",
    );
  if (!ready)
    return (
      <div className="loading">
        <span className="brand">
          <img className="brand-symbol" src={brandMark} alt="" />
          camofy<span>cloud</span>
        </span>
        <p>正在连接工作区…</p>
      </div>
    );
  if (!user)
    return (
      <Login
        onLogin={(u) => {
          setLoading(true);
          setUser(u);
        }}
      />
    );
  if (location.pathname === "/authorize")
    return (
      <Authorize
        user={user}
        logout={() => {
          void api("/auth/logout", "POST")
            .then(() => setUser(null))
            .catch((e) => setError(e.message));
        }}
      />
    );
  const current = sections.find((s) =>
    location.pathname.startsWith(`/${s.key}`),
  );
  const allowedSections = sections.filter(
    (s) => s.key !== "proxies" || user.role === "admin",
  );
  return (
    <WorkspaceContext.Provider
      value={{
        resources,
        user,
        loading,
        busy,
        error,
        notice,
        connected,
        load,
        run,
        save,
      }}
    >
      <div className={`workspace ${mobile ? "menu-open" : ""}`}>
        {mobile && (
          <button
            className="nav-scrim"
            aria-label="关闭导航"
            onClick={() => setMobile(false)}
          />
        )}
        <aside className="sidebar">
          <Link
            to="/identities"
            className="brand"
            onClick={() => setMobile(false)}
          >
            <img className="brand-symbol" src={brandMark} alt="" />
            camofy<span className="brand-edition">CLOUD</span>
          </Link>
          <div className="workspace-switch">
            <span className="workspace-avatar">
              {user.email[0].toUpperCase()}
            </span>
            <div>
              <strong>个人工作区</strong>
              <small>{user.nickname || user.email}</small>
            </div>
            <Icon name="shield" size={16} />
          </div>
          <div className="nav-caption">配置管理</div>
          <nav aria-label="主导航">
            <NavLink
              to="/store"
              className={({ isActive }) =>
                `nav-item ${isActive ? "active" : ""}`
              }
              onClick={() => setMobile(false)}
            >
              <Icon name="layers" />
              <span>Profile 商店</span>
            </NavLink>
            {sections
              .filter((s) => s.key !== "proxies")
              .map((s, i) => (
                <NavLink
                  key={s.key}
                  to={`/${s.key}`}
                  onClick={() => setMobile(false)}
                  className={({ isActive }) =>
                    `nav-item ${isActive ? "active" : ""} ${i === 3 ? "nav-separator" : ""}`
                  }
                >
                  <Icon name={s.icon} />
                  <span>{s.name}</span>
                  <small>
                    {resources.filter((r) => sectionOf(r) === s.key).length}
                  </small>
                </NavLink>
              ))}
            {user.role === "admin" && (
              <>
                <div className="nav-caption">系统管理</div>
                <NavLink
                  to="/proxies"
                  className={({ isActive }) =>
                    `nav-item ${isActive ? "active" : ""}`
                  }
                  onClick={() => setMobile(false)}
                >
                  <Icon name="route" />
                  <span>订阅出口</span>
                </NavLink>
              </>
            )}
          </nav>
          <div className="sidebar-note">
            <a
              href={
                import.meta.env.DEV && import.meta.env.MODE === "design"
                  ? "http://127.0.0.1:18741/"
                  : "https://camofy.app/"
              }
              target="_blank"
              rel="noreferrer"
            >
              Camofy 官网 <Icon name="arrow" size={14} />
            </a>
          </div>
          <div className="sidebar-account">
            <span className="account-avatar">
              {user.email[0].toUpperCase()}
            </span>
            <Link
              to="/account"
              title="个人资料"
              onClick={() => setMobile(false)}
            >
              {user.nickname || "个人资料"}
            </Link>
            <button
              className="icon-button"
              aria-label="退出登录"
              onClick={() => {
                void api("/auth/logout", "POST")
                  .then(() => {
                    setUser(null);
                    setResources([]);
                  })
                  .catch((e) => setError(e.message));
              }}
            >
              <Icon name="arrow" size={16} />
            </button>
          </div>
        </aside>
        <div className="main-column">
          <div className="topbar">
            <button
              className="mobile-toggle icon-button"
              aria-label="打开导航"
              onClick={() => setMobile(true)}
            >
              <Icon name="menu" />
            </button>
            <div className="topbar-breadcrumb">
              工作区<span>/</span>
              {current?.name ??
                (location.pathname.startsWith("/store")
                  ? "Profile 商店"
                  : location.pathname === "/account"
                    ? "个人资料"
                    : "页面")}
            </div>
            <div className={`live-indicator ${connected ? "connected" : ""}`}>
              <i />
              {import.meta.env.DEV && import.meta.env.MODE === "design"
                ? "本地演示 · 无线上连接"
                : connected
                  ? "变更推送已连接"
                  : "等待连接 · 定时同步可用"}
            </div>
          </div>
          <main className="workspace-content">
            {error && (
              <div className="banner error" role="alert">
                <div>
                  <strong>操作未完成</strong>
                  <span>{error}</span>
                </div>
                <button
                  className="icon-button"
                  aria-label="关闭错误"
                  onClick={() => setError("")}
                >
                  <Icon name="close" />
                </button>
              </div>
            )}
            {loading ? (
              <div className="skeleton-page" aria-label="正在加载">
                <div />
                <div />
                <div />
              </div>
            ) : (
              <Routes>
                <Route path="/store" element={<StorePage />} />
                <Route path="/store/:slug" element={<StoreDetail />} />
                <Route
                  path="/account"
                  element={<AccountPage user={user} onChange={setUser} />}
                />
                <Route
                  path="/"
                  element={<Navigate to="/identities" replace />}
                />
                <Route
                  path="/tokens"
                  element={<Navigate to="/identities" replace />}
                />
                {allowedSections.map((s) => (
                  <Route
                    key={s.key}
                    path={`/${s.key}`}
                    element={<CollectionPage section={s.key} />}
                  />
                ))}
                {allowedSections.flatMap((s) => [
                  <Route
                    key={`${s.key}-new`}
                    path={`/${s.key}/new`}
                    element={<EditPage section={s.key} fresh />}
                  />,
                  <Route
                    key={`${s.key}-edit`}
                    path={`/${s.key}/:id/edit`}
                    element={<EditPage section={s.key} />}
                  />,
                  <Route
                    key={`${s.key}-detail`}
                    path={`/${s.key}/:id`}
                    element={<DetailPage section={s.key} />}
                  />,
                ])}
                <Route path="*" element={<NotFound />} />
              </Routes>
            )}
            <footer className="workspace-footer">
              <span>CAMOFY CLOUD</span>为你的所有设备，组织同一份网络配置。
            </footer>
          </main>
        </div>
        {notice && (
          <div className="toast" role="status">
            <Icon name="check" />
            {notice}
          </div>
        )}
      </div>
    </WorkspaceContext.Provider>
  );
}
const router = createBrowserRouter([
  { path: "*", element: <CloudWorkspace /> },
]);
export default function CloudApp() {
  return <RouterProvider router={router} />;
}
