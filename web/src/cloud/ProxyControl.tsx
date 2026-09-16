import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { api, displayTime, type Resource } from "./model";
import { Panel, PanelBody, Icon } from "./ui";
import "./proxy-control.css";

type Group = {
  name: string;
  kind: string;
  members: string[];
  now?: string;
  dynamic: boolean;
};
type State = {
  status?: string;
  errors?: Record<string, string>;
  received_at?: number;
  sampled_at?: number;
  pending_local?: boolean;
  selection_version?: number;
  override_version?: number;
};
type Job = {
  id: string;
  method: string;
  status: string;
  created_at: number;
  params?: { name?: string };
  result?: {
    error?: string;
    value?: { name?: string; delay?: number; sampled_at?: number };
  };
};
type Event = {
  id: string;
  created_at: number;
  group: string;
  from?: string;
  to?: string;
  source: string;
  status: string;
  detail?: string;
};
type View = {
  identity_id?: string;
  identity_name?: string;
  groups?: Group[];
  selections?: Record<string, string>;
  overrides?: Record<string, string>;
  version: number;
  state?: State;
  reported?: { protocol?: number; core_state?: string };
  jobs?: Job[];
  events?: Event[];
  devices?: {
    id: string;
    name: string;
    reported?: { proxy_state?: State };
    overrides?: Record<string, string>;
  }[];
};
const labels: Record<string, string> = {
  queued: "等待设备",
  pending: "等待设备确认",
  executing: "执行中",
  succeeded: "已完成",
  failed: "失败",
  expired: "已过期",
  superseded: "已被后续选择替代",
  unknown: "结果未知",
  saved: "共享选择已保存",
  applied: "已生效",
  partial: "部分未生效",
  stopped: "内核已停止",
  stopping: "正在停止",
  unavailable: "内核未就绪",
  running: "内核运行中",
};
const methods: Record<string, string> = {
  "proxies.delay": "节点测速",
  "core.start": "启动内核",
  "core.stop": "停止内核",
  "core.restart": "重启内核",
};
const errors: Record<string, string> = {
  group_missing: "分组已不存在",
  node_missing: "节点不存在或尚未加载",
  not_selectable: "此分组由自动策略控制",
  apply_failed: "切换暂未成功，设备将重试",
  readback_mismatch: "等待设备回读确认",
  pending: "等待设备应用",
};

export function ProxyControl({ r }: { r: Resource }) {
  const device = r.kind === "device";
  const [view, setView] = useState<View>();
  const [params, setParams] = useSearchParams();
  const [groupQuery, setGroupQuery] = useState("");
  const [nodeQuery, setNodeQuery] = useState("");
  const [limit, setLimit] = useState(60);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [selectionBusy, setSelectionBusy] = useState(false);
  const [working, setWorking] = useState<Record<string, boolean>>({});
  const [filter, setFilter] = useState("all");
  const [optimistic, setOptimistic] = useState<{
    group: string;
    node?: string;
  }>();
  const inFlight = useRef(false);
  const mutation = useRef(0);
  const mounted = useRef(true);
  const automaticRefresh = useRef(false);
  const load = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    const startedAt=mutation.current;
    try {
      const next = await api<View>(`/resources/${r.id}/proxies`);
      if (mounted.current && startedAt===mutation.current) {
        setView(next);
        setError("");
      }
    } catch (e) {
      if (mounted.current) setError((e as Error).message);
    } finally {
      inFlight.current = false;
    }
  }, [r.id]);
  useEffect(() => {
    mounted.current = true;
    void load();
    const timer = setInterval(() => {
      if (!document.hidden) void load();
    }, 2000);
    const visible = () => {
      if (!document.hidden) void load();
    };
    document.addEventListener("visibilitychange", visible);
    return () => {
      mounted.current = false;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", visible);
    };
  }, [load]);
  // Recover a lost first report once, without requiring a manual refresh or flooding RPC.
  useEffect(() => {
    if (
      !device ||
      !view ||
      automaticRefresh.current ||
      view.reported?.protocol !== 2 ||
      view.reported.core_state !== "running" ||
      view.state?.sampled_at
    )
      return;
    automaticRefresh.current = true;
    void api(`/devices/${r.id}/rpc`, "POST", {
      method: "proxies.list",
      params: null,
      idempotency_key: crypto.randomUUID(),
    })
      .then(() => load())
      .catch((e) => setError((e as Error).message));
  }, [device, view, r.id, load]);
  async function change(group: string, node?: string) {
    if (!view || selectionBusy || device) return;
    setSelectionBusy(true);
    mutation.current+=1;
    setOptimistic({ group, node });
    setError("");
    setNotice("");
    const selections = { ...view.selections };
    if (node === undefined) delete selections[group];
    else selections[group] = node;
    try {
      const result=await api<{version:number}>(`/resources/${r.id}/selections`, "PUT", {
        expected_version: view.version,
        selections,
      });
      setView(previous=>previous?{...previous,selections,version:result.version}:previous);
      await load();
      setNotice(
        device
          ? "选择已保存，正在等待设备确认。"
          : "共享选择已保存，跟随此身份的设备将自动同步。",
      );
    } catch (e) {
      await load();
      setError((e as Error).message);
    } finally {
      setSelectionBusy(false);
      setOptimistic(undefined);
    }
  }
  async function rpc(method: string, name?: string) {
    const key = name ?? method;
    setWorking((v) => ({ ...v, [key]: true }));
    setError("");
    try {
      await api(`/devices/${r.id}/rpc`, "POST", {
        method,
        params: name ? { name } : null,
        idempotency_key: crypto.randomUUID(),
      });
      await load();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setWorking((v) => ({ ...v, [key]: false }));
    }
  }
  function navigate(key: string, value: string) {
    const next = new URLSearchParams(params);
    next.set(key, value);
    setParams(next, { replace: true });
  }
  const all = view?.groups ?? [];
  const filtered = all.filter((g) =>
    g.name.toLowerCase().includes(groupQuery.toLowerCase()),
  );
  const selected =
    filtered.find((g) => g.name === params.get("group")) ??
    filtered.find((g) => g.name === "Proxies") ??
    filtered.find((g) => g.name !== "GLOBAL") ??
    filtered[0];
  const activity = params.get("proxyView") === "activity";
  const stale =
    device &&
    (!view?.state?.received_at ||
      Date.now() / 1000 - view.state.received_at > 360);
  const core = view?.reported?.core_state;
  const desired = selected ? view?.selections?.[selected.name] : undefined;
  const selectable =
    selected && (selected.kind === "Selector" || selected.kind === "select");
  const pending =
    selected &&
    device &&
    desired !== undefined &&
    (desired !== selected.now ||
      view?.state?.selection_version !== view?.version);
  const groupError = selected && view?.state?.errors?.[selected.name];
  const members =
    selected?.members.filter((n) =>
      n.toLowerCase().includes(nodeQuery.toLowerCase()),
    ) ?? [];
  if (params.get("order") === "name")
    members.sort((a, b) => a.localeCompare(b, "en", { numeric: true }));
  const route: string[] = [];
  if (selected && device) {
    let cursor: Group | undefined = selected;
    const seen = new Set<string>();
    while (cursor && !seen.has(cursor.name)) {
      seen.add(cursor.name);
      route.push(cursor.name);
      if (!cursor.now) break;
      const next = all.find((g) => g.name === cursor!.now);
      if (!next) {
        route.push(cursor.now);
        break;
      }
      cursor = next;
    }
  }
  const jobs = view?.jobs ?? [];
  const operationPending = jobs.some(
    (j) =>
      j.method.startsWith("core.") &&
      ["queued", "executing"].includes(j.status),
  );
  const timeline = [
    ...(view?.events ?? []).map((e) => ({
      id: e.id,
      time: e.created_at,
      type: "selection",
      title: e.to ? "切换节点" : "恢复默认选择",
      status: e.status,
      detail: `${e.group} · ${e.from ?? "配置默认"} → ${e.to ?? "配置默认"}`,
      source: e.source === "local" ? "设备本地" : "云端",
      error: e.detail ? (errors[e.detail] ?? e.detail) : "",
    })),
    ...jobs
      .filter((j) => j.method !== "proxies.list" && j.method !== "core.status")
      .map((j) => ({
        id: j.id,
        time: j.created_at,
        type: j.method === "proxies.delay" ? "delay" : "core",
        title: methods[j.method] ?? "设备操作",
        status: j.status,
        detail:
          j.method === "proxies.delay"
            ? `${j.params?.name ?? j.result?.value?.name ?? "节点"}${j.result?.value?.delay !== undefined ? ` · ${j.result.value.delay} ms` : ""}`
            : "",
        source: "云端",
        error:
          j.status === "failed"
            ? j.method === "proxies.delay"
              ? "连接失败或测速超时，可重试。"
              : "设备未完成操作，请查看本地状态。"
            : "",
      })),
  ].sort((a, b) => b.time - a.time);
  const visibleEvents = timeline.filter(
    (e) => filter === "all" || e.type === filter,
  );
  return (
    <div className="proxy-control">
      <div className="proxy-topbar">
        <div>
          <strong>
            {device
              ? stale
                ? "等待设备连接"
                : (labels[core ?? ""] ?? "正在同步设备")
              : "身份共享选择"}
          </strong>
          <span className="muted">
            {device
              ? `自动同步 · ${view?.state?.sampled_at ? displayTime(view.state.sampled_at) : "等待首次快照"}`
              : "影响所有跟随此身份的 Agent"}
          </span>
        </div>
        <div className="proxy-top-actions">
          <button
            disabled={!!working["proxies.list"]}
            onClick={() => (device ? void rpc("proxies.list") : void load())}
          >
            <Icon name="refresh" size={14} />
            {working["proxies.list"] ? "同步中…" : "同步状态"}
          </button>
          {device && (
            <details className="proxy-core-menu">
              <summary>内核操作</summary>
              <div>
                {(
                  [
                    ["core.start", "启动内核"],
                    ["core.stop", "停止内核"],
                    ["core.restart", "重启内核"],
                  ] as const
                ).map(([method, label]) => (
                  <button
                    key={method}
                    disabled={
                      operationPending ||
                      !!working[method] ||
                      view?.reported?.protocol !== 2 ||
                      (method === "core.start" && core === "running") ||
                      (method === "core.stop" && core === "stopped")
                    }
                    onClick={() => {
                      if (
                        confirm(
                          `${label}可能中断代理连接。设备会先优雅退出并清理网络规则，是否继续？`,
                        )
                      )
                        void rpc(method);
                    }}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </details>
          )}
        </div>
      </div>
      <div
        className="proxy-section-tabs"
        role="tablist"
        aria-label="代理工作区"
      >
        <button
          role="tab"
          aria-selected={!activity}
          onClick={() => navigate("proxyView", "nodes")}
        >
          {device ? "节点预览" : "节点选择"} <span>{all.length} 个分组</span>
        </button>
        <button
          role="tab"
          aria-selected={activity}
          onClick={() => navigate("proxyView", "activity")}
        >
          操作记录 <span>{timeline.length}</span>
        </button>
      </div>
      {error && (
        <div className="proxy-feedback is-error" role="alert">
          {error}
          <button onClick={() => void load()}>重试</button>
        </div>
      )}
      {notice && (
        <div className="proxy-feedback" role="status">
          {notice}
          <button aria-label="关闭提示" onClick={() => setNotice("")}>
            ×
          </button>
        </div>
      )}
      {!view && !error && (
        <div className="proxy-empty" role="status">
          正在读取设备与分组…
        </div>
      )}
      {activity ? (
        <Panel
          title="操作记录"
          actions={
            <select
              aria-label="筛选操作类型"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
            >
              <option value="all">所有操作</option>
              <option value="selection">节点切换</option>
              <option value="delay">节点测速</option>
              <option value="core">内核操作</option>
            </select>
          }
        >
          <PanelBody>
            <p className="muted">
              记录选择变更、设备确认和测速结果，不采集代理流量日志。自动状态查询不在此刷屏。
            </p>
            <ol className="proxy-timeline">
              {visibleEvents.slice(0, 80).map((e) => (
                <li key={e.id}>
                  <div>
                    <strong>{e.title}</strong>
                    <span className={`proxy-result result-${e.status}`}>
                      {labels[e.status] ?? e.status}
                    </span>
                  </div>
                  <p>{e.detail}</p>
                  {e.error && <p className="error-text">{e.error}</p>}
                  <small>
                    {displayTime(e.time)} · {e.source}
                  </small>
                </li>
              ))}
            </ol>
            {!visibleEvents.length && (
              <div className="proxy-empty">
                暂无操作记录。新发生的节点切换会显示在这里。
              </div>
            )}
          </PanelBody>
        </Panel>
      ) : view && all.length ? (
        <div className="proxy-workspace">
          <aside className="proxy-group-nav" aria-label="代理分组">
            <label className="proxy-search">
              查找分组
              <input
                type="search"
                placeholder="搜索分组"
                value={groupQuery}
                onChange={(e) => {
                  setGroupQuery(e.target.value);
                  setNodeQuery("");
                  setLimit(60);
                }}
              />
            </label>
            <div className="proxy-group-options">
              {filtered.map((g) => (
                <button
                  key={g.name}
                  aria-current={selected?.name === g.name ? "true" : undefined}
                  onClick={() => {
                    navigate("group", g.name);
                    setNodeQuery("");
                    setLimit(60);
                    setNotice("");
                  }}
                >
                  <span>
                    <strong>{g.name}</strong>
                    <small>
                      {device
                        ? (g.now ?? "等待快照")
                        : (view.selections?.[g.name] ?? "客户端默认")}
                    </small>
                  </span>
                  <span className="proxy-group-count">{g.members.length}</span>
                </button>
              ))}
            </div>
            {!filtered.length && <p className="muted">没有匹配的分组</p>}
          </aside>
          <div className="proxy-main">
            {selected && (
              <Panel
                title={selected.name}
                actions={
                  <span className="chip">
                    {selectable ? "手动选择" : "自动策略"} ·{" "}
                    {selected.members.length}
                  </span>
                }
              >
                <PanelBody>
                  <div className="proxy-selection-summary">
                    <div>
                      <small>{device ? "设备当前选择" : "共享选择"}</small>
                      <strong>
                        {device
                          ? (selected.now ?? "等待回读")
                          : (desired ?? "客户端默认")}
                      </strong>
                      <span>
                        {device
                          ? `只读预览 · 跟随 ${view.identity_name ?? "身份"}`
                          : "绑定设备自动同步；订阅客户端需更新订阅"}
                      </span>
                    </div>
                    {device && (
                      <Link
                        className="button"
                        to={`/identities/${view.identity_id}?tab=proxies&group=${encodeURIComponent(selected.name)}`}
                      >
                        前往身份调整
                      </Link>
                    )}
                    {!device && view.selections?.[selected.name] && (
                      <button
                        className="quiet"
                        disabled={selectionBusy}
                        onClick={() => void change(selected.name)}
                      >
                        恢复配置默认
                      </button>
                    )}
                  </div>
                  {device && route.length > 1 && (
                    <p className="proxy-route" aria-label="实际出口链路">
                      {route.join(" → ")}
                    </p>
                  )}
                  {pending && (
                    <p className="proxy-inline-status" role="status">
                      期望切换到 {desired} ·{" "}
                      {stale
                        ? "设备离线，连接后自动应用"
                        : core === "stopped"
                          ? "已保存，内核启动后应用"
                          : "等待设备确认"}
                    </p>
                  )}
                  {groupError && (
                    <p role="alert" className="error-text">
                      {errors[groupError] ?? groupError}
                    </p>
                  )}
                  {desired && !selected.members.includes(desired) && (
                    <p className="error-text">
                      期望节点已不在当前分组中，请重新选择。
                    </p>
                  )}
                  <div className="proxy-node-tools">
                    <label className="proxy-search">
                      <input
                        type="search"
                        placeholder="搜索节点名称…"
                        aria-label="搜索当前分组节点"
                        value={nodeQuery}
                        onChange={(e) => {
                          setNodeQuery(e.target.value);
                          setLimit(60);
                        }}
                      />
                    </label>
                    <small>{members.length} 个节点</small>
                    <select
                      aria-label="节点排序"
                      value={params.get("order") ?? "config"}
                      onChange={(e) => navigate("order", e.target.value)}
                    >
                      <option value="config">配置顺序</option>
                      <option value="name">名称排序</option>
                    </select>
                  </div>
                  {!selectable && (
                    <p className="muted">
                      此分组由内核自动选择，下方仅展示实际状态。
                    </p>
                  )}
                  <div className="proxy-node-grid">
                    {members.slice(0, limit).map((node) => {
                      const job = [...jobs]
                        .reverse()
                        .find(
                          (j) =>
                            j.method === "proxies.delay" &&
                            (j.params?.name ?? j.result?.value?.name) === node,
                        );
                      const measuring =
                        !!working[node] ||
                        (!!job && ["queued", "executing"].includes(job.status));
                      const value =
                        job?.status === "succeeded"
                          ? job.result?.value
                          : undefined;
                      const actual = device && selected.now === node;
                      const target =
                        optimistic?.group === selected.name
                          ? optimistic.node === node
                          : desired === node;
                      return (
                        <div
                          className={`proxy-node ${actual || (!device && target) ? "is-selected" : ""} ${target && !actual ? "is-pending" : ""}`}
                          key={node}
                        >
                          {device ? (
                            <div className="proxy-node-select">
                              <strong>{node}</strong>
                              <small>
                                {actual
                                  ? "✓ 设备实际使用"
                                  : target
                                    ? "身份期望 · 等待同步"
                                    : "只读预览"}
                              </small>
                            </div>
                          ) : (
                            <button
                              className="proxy-node-select"
                              aria-pressed={!!(device ? actual : target)}
                              disabled={
                                !selectable ||
                                selectionBusy ||
                                (device ? actual && !pending : target)
                              }
                              onClick={() => void change(selected.name, node)}
                            >
                              <strong>{node}</strong>
                              <small>
                                {actual
                                  ? "✓ 当前使用"
                                  : target
                                    ? device
                                      ? "等待确认"
                                      : "共享首选"
                                    : !selectable
                                      ? "自动选择"
                                      : "点击选择"}
                              </small>
                            </button>
                          )}
                          {device && (
                            <button
                              className={`proxy-delay ${job?.status === "failed" ? "is-failed" : ""}`}
                              disabled={
                                measuring ||
                                stale ||
                                core !== "running" ||
                                view.reported?.protocol !== 2
                              }
                              title={
                                value
                                  ? `此设备 · ${displayTime(value.sampled_at)}`
                                  : "从当前设备测速"
                              }
                              aria-label={`测速 ${node}`}
                              onClick={() => void rpc("proxies.delay", node)}
                            >
                              {measuring
                                ? "测速中…"
                                : value?.delay !== undefined
                                  ? `${value.delay} ms`
                                  : job?.status === "failed"
                                    ? "超时 · 重试"
                                    : "测速"}
                            </button>
                          )}
                        </div>
                      );
                    })}
                  </div>
                  {!members.length && (
                    <div className="proxy-empty">
                      没有匹配的节点
                      {selected.dynamic ? "；动态节点加载后将自动出现" : ""}
                    </div>
                  )}
                  {members.length > limit && (
                    <button
                      className="proxy-show-more"
                      onClick={() => setLimit((n) => n + 60)}
                    >
                      再显示 {Math.min(60, members.length - limit)} 个节点
                    </button>
                  )}
                  <p className="proxy-footnote">
                    {device
                      ? "只读预览，节点选择统一在身份页面修改。"
                      : "身份选择会同步至跟随设备。"}
                    不会主动断开已有连接。
                    {selected.dynamic ? "动态节点以设备上报为准。" : ""}
                  </p>
                </PanelBody>
              </Panel>
            )}
          </div>
        </div>
      ) : (
        view && (
          <div className="proxy-empty" role="status">
            <strong>
              {core === "stopped"
                ? "内核已停止"
                : stale
                  ? "等待设备上线"
                  : "正在等待分组快照"}
            </strong>
            <p>
              {device
                ? core === "stopped"
                  ? "启动内核后，分组将自动显示，无需手动刷新。"
                  : "设备应用身份后会自动上报。此页会持续同步；网络恢复后无需重新打开。"
                : "当前身份尚未配置代理分组。"}
            </p>
          </div>
        )
      )}
      {!device && !!view?.devices?.length && (
        <details className="proxy-device-summary">
          <summary>设备同步情况 · {view.devices.length} 台</summary>
          {view.devices.map((d) => {
            const s = d.reported?.proxy_state;
            const offline =
              !s?.received_at || Date.now() / 1000 - s.received_at > 360;
            return (
              <div key={d.id}>
                <a href={`/devices/${d.id}?tab=proxies`}>{d.name}</a>
                <span>
                  {offline
                    ? "离线 / 状态过期"
                    : s?.selection_version !== view.version
                      ? "等待同步"
                      : (labels[s?.status ?? ""] ?? "等待确认")}
                </span>
              </div>
            );
          })}
        </details>
      )}
    </div>
  );
}
