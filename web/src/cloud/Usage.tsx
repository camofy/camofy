import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { api, displayTime, type Resource, type UsageSummary } from "./model";

const states: Record<string, string> = {
  ok: "数据完整", stale: "使用上次快照", partial: "数据不完整", empty: "无订阅套餐",
  unavailable: "等待首次统计", overflow: "用量数值异常", missing: "上游未提供用量",
  invalid: "用量格式无效", incomplete: "额度字段不完整", outdated: "快照已过时",
  source_changed: "来源已变更", conflict: "同额度池数据冲突",
};
function bytes(value?: string | null): string {
  if (value == null) return "—";
  const n = BigInt(value);
  const units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
  let divisor = 1n, unit = 0;
  while (n >= divisor * 1024n && unit < units.length - 1) { divisor *= 1024n; unit++; }
  return `${(Number(n * 100n / divisor) / 100).toLocaleString(undefined, { maximumFractionDigits: 2 })} ${units[unit]}`;
}
const used = (u: { upload: string | null; download: string | null }) => u.upload !== null && u.download !== null ? (BigInt(u.upload) + BigInt(u.download)).toString() : null;
function Meter({ amount, total }: { amount: string | null; total: string | null }) {
  if (amount === null || total === null || BigInt(total) === 0n) return null;
  const percent = Math.min(100, Number(BigInt(amount) * 10000n / BigInt(total)) / 100);
  return <div className={`usage-meter ${percent >= 90 ? "near-limit" : ""}`} role="meter" aria-label="已用流量比例" aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}><span style={{ width: `${percent}%` }} /></div>;
}
export function UsageCompact({ summary: u }: { summary?: UsageSummary }) {
  if (!u || u.known_pools === 0 || u.status === "overflow") return <span className="muted usage-compact">{states[u?.status ?? "unavailable"]}</span>;
  return <div className="usage-compact"><strong>{bytes(used(u))}<span> / {bytes(u.total)}</span></strong><Meter amount={used(u)} total={u.total} />{u.status !== "ok" && <small>{states[u.status]} · {u.known_pools}/{u.total_pools} 套餐</small>}</div>;
}
export function UsagePanel({ r }: { r: Resource }) {
  const u = r.data.usage_summary;
  const identity = r.kind === "bundle";
  const complete = u && ["ok", "stale"].includes(u.status);
  const known = u && u.known_pools > 0 && u.status !== "overflow";
  return <section className="panel usage-panel" aria-label={identity ? "套餐额度汇总" : "订阅使用量"}>
    <div className="panel-heading"><div><span className="eyebrow">SUBSCRIPTION USAGE</span><h2>{identity ? "套餐额度汇总" : "订阅使用量"}</h2></div><span className={`chip ${complete ? "" : "disabled"}`}>{states[u?.status ?? "unavailable"]}</span></div>
    <div className="usage-headline"><div><span className="muted">{complete ? "已用流量" : "已知小计"}</span><div className="usage-value">{known ? bytes(used(u)) : "—"}<small>/ {known ? bytes(u.total) : "—"}</small></div></div><div className="usage-coverage"><strong>{u?.known_pools ?? 0}<span> / {u?.total_pools ?? 0}</span></strong><small>套餐有可用数据</small></div></div>
    {known && <Meter amount={used(u)} total={u.total} />}
    <dl className="usage-metrics">
      <div><dt>上传</dt><dd>{known ? bytes(u.upload) : "—"}</dd></div>
      <div><dt>下载</dt><dd>{known ? bytes(u.download) : "—"}</dd></div>
      <div><dt>套餐剩余之和</dt><dd>{known ? bytes(u.remaining) : "—"}</dd></div>
      <div><dt>{identity ? "最早到期（非整体失效）" : "到期时间"}</dt><dd>{displayTime(u?.next_expire ?? undefined)}</dd></div>
    </dl>
    {!complete && <p className="usage-warning">{u?.status === "empty" ? "此身份没有启用的订阅源，配置 Profile 不计入用量。" : "用量信息尚不完整，客户端暂不显示总额度；不会将缺失数据当作零。"}</p>}
    <p className="footnote">来自机场，包含其他客户端用量，并非本设备实时流量。不同套餐额度不能互相借用。{u?.updated_at ? ` 最旧有效快照：${displayTime(u.updated_at)}。` : " 请刷新订阅以获取用量。"}</p>
    {!!u?.pools.length && <div className="usage-pools">{u.pools.map((p) => <div className="usage-pool" key={p.profile_ids.join(":")}>
      <div><div className="usage-pool-names">{p.names.map((name, i) => <Link key={p.profile_ids[i]} to={`/subscriptions/${p.profile_ids[i]}`}>{name}</Link>)}</div><small>{states[p.status]}{p.profile_ids.length > 1 ? " · 共用额度，仅计一次" : ""}{p.expired ? " · 已到期" : ""}</small></div>
      <div><strong>{bytes(used(p))} / {bytes(p.total)}</strong><small>到期 {displayTime(p.expire ?? undefined)} · 快照 {displayTime(p.updated_at ?? undefined)}</small></div>
    </div>)}</div>}
    {!identity && <p className="footnote">共用额度池：{r.data.usage_pool || "自动按相同订阅 URL 去重"}。同一套餐不同链接可在编辑页填写相同额度池名称。</p>}
  </section>;
}
type Entry = { id: string; started_at: number; finished_at: number | null; duration_ms: number | null; reason: string; status: string; error_code: string | null; message: string | null; usage_status: string | null };
type HistoryPage = { items: Entry[]; next_cursor: string | null };
export function RefreshHistory({ r }: { r: Resource }) {
  const [cursors, setCursors] = useState<string[]>([]);
  const [reload, setReload] = useState(0);
  const [result, setResult] = useState<{ key: string; page?: HistoryPage; error?: string }>();
  const cursor = cursors.at(-1) ?? "";
  const key = `${r.id}:${cursor}:${reload}`;
  useEffect(() => {
    let active = true;
    const load = () => void api<HistoryPage>(`/profiles/${r.id}/history${cursor ? `?before=${cursor}` : ""}`)
      .then((page) => { if (active) setResult({ key, page }); })
      .catch((e) => { if (active) setResult({ key, error: e.message }); });
    load();
    const timer = setInterval(load, 5000);
    return () => { active = false; clearInterval(timer); };
  }, [r.id, cursor, key]);
  const page = result?.key === key ? result.page : undefined;
  const reasons: Record<string, string> = { manual: "手动刷新", scheduled: "定时刷新", initial: "首次拉取", settings: "配置变更" };
  const statuses: Record<string, string> = { running: "刷新中", ok: "成功", error: "失败", discarded: "已丢弃" };
  return <section className="panel refresh-history"><div className="panel-heading"><div><h2>订阅刷新历史</h2><p className="muted">保留最近 30 天、最多 100 条。从本功能上线开始记录，失败不覆盖上次成功配置。</p></div><button onClick={() => setReload((n) => n + 1)}>刷新记录</button></div>
    {result?.key === key && result.error ? <p role="alert" className="inline-error">{result.error}</p> : !page ? <p role="status" className="panel-message">正在加载刷新记录…</p> : !page.items.length ? <p className="panel-message">暂无刷新记录。可点击页面右上角「立即刷新」。</p> : <div className="table-wrap"><table className="resource-table"><thead><tr><th>开始时间 / 触发方式</th><th>结果</th><th>耗时</th><th>详情</th></tr></thead><tbody>{page.items.map((e) => <tr key={e.id}><td>{displayTime(e.started_at)}<small>{reasons[e.reason] ?? e.reason}</small></td><td><span className={`chip ${e.status === "error" ? "history-error" : ""}`}>{statuses[e.status] ?? e.status}</span></td><td>{e.duration_ms === null ? "进行中" : `${(e.duration_ms / 1000).toFixed(2)} 秒`}</td><td className="history-message">{e.message || (e.status === "ok" ? "已更新订阅内容" : "正在请求上游，请稍候")}<small>{e.error_code ? `诊断代码：${e.error_code} · ` : ""}用量：{states[e.usage_status ?? "missing"] ?? "尚未读取"}</small></td></tr>)}</tbody></table></div>}
    <div className="history-pagination"><button disabled={!cursors.length} onClick={() => setCursors((v) => v.slice(0, -1))}>上一页</button><span>第 {cursors.length + 1} 页</span><button disabled={!page?.next_cursor} onClick={() => { if (page?.next_cursor) setCursors((v) => [...v, page.next_cursor!]); }}>下一页</button></div>
  </section>;
}
