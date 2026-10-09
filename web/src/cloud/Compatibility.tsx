import { useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { api, type Data, type Resource } from "./model";
import { useWorkspace } from "./context";
import { ConfigPreview, FieldActionRow, Panel, PanelBody } from "./ui";
import "./compatibility.css";

type Policy = NonNullable<Data["node_filter"]>;
type Profile = { name: string; protocols: string[]; unsupported_protocols?: string[]; complete_protocols: boolean; features: Record<string, boolean>; evidence: string[] };
type Client = { id: string; name: string; format: string; notes: string; releases: { version: string; profile: string | null; evidence: string; confidence: string; prerelease?: boolean }[]; ranges: { min: string; max: string; profile: string }[] };
type Matrix = { version: string; checked_at: string; protocols: string[]; clients: Client[]; profiles: Record<string, Profile> };
type Preview = {
  content?: string; error?: string; format?: string;
  report: { client: { name?: string; version?: string; confidence: string }; before: number; retained: number; removed: number; repaired_references: number; blocked_groups: string[]; warnings: string[]; unknown_capabilities: string[]; exclusions: { name: string; protocol: string; reason: string; capability: string }[] };
};
const defaultPolicy: Policy = { auto: true, exclude_types: [] };
const reasons: Record<string, string> = { manual: "指定类型排除", unsupported_protocol: "客户端不支持此协议", unsupported_feature: "客户端不支持此功能", unsupported_output: "导出格式无法无损表达", dependency: "上游节点已排除" };
const formatNames: Record<string, string> = { router: "完整 YAML", clash: "Clash YAML", shadowrocket: "Shadowrocket 完整配置", "shadowrocket-nodes": "Shadowrocket 节点订阅" };
const confidence: Record<string, string> = { verified: "已核实内核", bundled: "官方内置内核", documented: "官方版本说明", unknown: "能力未确认" };
const message = (e: unknown) => e instanceof Error ? e.message : "请求失败，请重试。";

export function Compatibility({ r }: { r: Resource }) {
  const [params, setParams] = useSearchParams();
  const view = ["policy", "preview", "matrix"].includes(params.get("compat_view") ?? "") ? params.get("compat_view")! : "policy";
  const [matrix, setMatrix] = useState<Matrix>();
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let active = true;
    api<Matrix>("/client-compatibility").then(value => { if (active) setMatrix(value); }).catch(e => { if (active) setError(message(e)); });
    return () => { active = false; };
  }, [attempt]);
  function navigate(next: string) {
    const nextParams = new URLSearchParams(params);
    nextParams.set("compat_view", next);
    setParams(nextParams);
  }
  return <div className="settings-stack compatibility">
    <nav className="compat-nav" aria-label="客户端兼容功能">
      {[["policy", "过滤策略"], ["preview", "模拟客户端"], ["matrix", "兼容矩阵"]].map(([id, label]) => <button key={id} aria-current={view === id ? "page" : undefined} className={view === id ? "selected" : ""} onClick={() => navigate(id)}>{label}</button>)}
    </nav>
    {error && <p className="inline-error" role="alert">矩阵加载失败：{error}<button onClick={() => { setError(""); setAttempt(v => v + 1); }}>重试</button></p>}
    {view === "policy" && <PolicyPanel r={r} matrix={matrix} onPreview={() => navigate("preview")} />}
    {view === "preview" && <ClientPreview r={r} />}
    {view === "matrix" && (matrix ? <MatrixPanel matrix={matrix} /> : !error && <p role="status">正在加载兼容矩阵…</p>)}
  </div>;
}

function PolicyPanel({ r, matrix, onPreview }: { r: Resource; matrix?: Matrix; onPreview: () => void }) {
  const { save, busy } = useWorkspace();
  const [draft, setDraft] = useState<Policy | null>(null);
  const [base, setBase] = useState(r);
  const [extra, setExtra] = useState("");
  const [error, setError] = useState("");
  const policy = draft ?? r.data.node_filter ?? defaultPolicy;
  const protocols = [...new Set([...(matrix?.protocols ?? ["ss", "ssr", "vmess", "vless", "trojan", "hysteria", "hysteria2", "tuic", "wireguard", "snell", "mieru", "anytls", "http", "socks5"]), ...policy.exclude_types])].sort();
  async function commit() {
    if (!draft) return;
    const saved = await save({ ...base, data: { ...base.data, node_filter: draft } });
    if (saved) { setDraft(null); setError(""); }
  }
  return <Panel title="这个身份的过滤策略" description="同一订阅地址按客户端能力返回节点；手动排除始终优先生效。" actions={!draft && <button onClick={() => { setBase(r); setDraft(structuredClone(policy)); }}>编辑策略</button>}>
    <PanelBody>
      {draft ? <form className="compat-form" onSubmit={e => { e.preventDefault(); void commit(); }}>
        <label className="check"><input type="checkbox" checked={policy.auto} onChange={e => setDraft({ ...policy, auto: e.target.checked })} />自动排除不兼容节点</label>
        <p className="muted">根据 User-Agent 识别客户端及版本。只排除已确认不支持的协议或功能；未确认的能力保留节点，并可在模拟结果中查看。</p>
        <fieldset><legend>始终排除的节点类型</legend><div className="compat-types">
          {protocols.map(type => <label className="check" key={type}><input type="checkbox" checked={policy.exclude_types.includes(type)} onChange={e => setDraft({ ...policy, exclude_types: e.target.checked ? [...policy.exclude_types, type].sort() : policy.exclude_types.filter(v => v !== type) })} /><code>{type}</code></label>)}
        </div></fieldset>
        <FieldActionRow><label>其他协议标识<input value={extra} maxLength={40} onChange={e => setExtra(e.target.value)} placeholder="例如：custom-protocol" /></label><button type="button" disabled={!extra.trim()} onClick={() => {
          const value = extra.trim().toLowerCase();
          if (!/^[a-z0-9-]{1,40}$/.test(value)) { setError("请输入协议标识，只能包含小写字母、数字和连字符。"); return; }
          setDraft({ ...policy, exclude_types: [...new Set([...policy.exclude_types, value])].sort() }); setExtra(""); setError("");
        }}>添加类型</button></FieldActionRow>
        {error && <p className="inline-error" role="alert">{error}</p>}
        <p className="muted">保存会发布此身份的新版本。指定类型排除也会作用于关联设备；自动兼容只在订阅下载时应用。</p>
        <div className="form-actions"><button type="button" disabled={busy} onClick={() => { setDraft(null); setError(""); }}>取消</button><button className="primary" disabled={busy}>{busy ? "正在保存…" : "保存并发布"}</button></div>
      </form> : <>
        <dl className="compat-policy"><div><dt>自动兼容</dt><dd><span className={`status ${policy.auto ? "good" : "neutral"}`}><i />{policy.auto ? "已开启" : "已关闭"}</span></dd></div><div><dt>始终排除</dt><dd>{policy.exclude_types.length ? <div className="compat-tags">{policy.exclude_types.map(type => <code className="chip" key={type}>{type}</code>)}</div> : "未指定类型"}</dd></div></dl>
        <p className="muted">缺少 User-Agent、未知版本或自定义内核时，无法保证自动识别。可以按类型排除，或先模拟实际请求查看结果。</p>
        <FieldActionRow><button onClick={onPreview}>模拟客户端</button><p className="muted">{matrix ? `矩阵更新于 ${matrix.checked_at} · ${matrix.clients.length} 个客户端系列` : "正在读取矩阵版本…"}</p></FieldActionRow>
      </>}
    </PanelBody>
  </Panel>;
}

function ClientPreview({ r }: { r: Resource }) {
  const [params, setParams] = useSearchParams();
  const [ua, setUa] = useState("");
  const format = ["auto", ...Object.keys(formatNames)].includes(params.get("preview_format") ?? "") ? params.get("preview_format")! : "auto";
  function setFormat(value: string) { const next = new URLSearchParams(params); next.set("preview_format", value); setParams(next, { replace: true }); }
  const [result, setResult] = useState<Preview>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const sequence = useRef(0);
  useEffect(() => () => { sequence.current++; }, []);
  function invalidate() { sequence.current++; setResult(undefined); setError(""); setBusy(false); }
  async function preview() {
    const id = ++sequence.current;
    setBusy(true); setError(""); setResult(undefined);
    try {
      const response = await api<Preview>(`/bundles/${r.id}/compatibility-preview`, "POST", { user_agent: ua || null, format });
      if (id === sequence.current) setResult(response);
    } catch (e) { if (id === sequence.current) setError(message(e)); }
    finally { if (id === sequence.current) setBusy(false); }
  }
  return <>
    <Panel title="模拟客户端请求" description="使用当前配置和已保存的过滤策略预览，不修改订阅或设备。">
      <PanelBody><form className="compat-form" onSubmit={e => { e.preventDefault(); void preview(); }}>
        <label>User-Agent<input value={ua} maxLength={512} onChange={e => { invalidate(); setUa(e.target.value); }} placeholder="留空可模拟未提供 User-Agent 的客户端" /></label>
        <FieldActionRow><label>快速示例<select value="" onChange={e => { invalidate(); setUa(e.target.value); }}><option value="" disabled>选择一个公开客户端版本</option><option value="ClashMetaForAndroid/2.10.2.Meta">Clash Meta for Android 2.10.2</option><option value="mihomo/1.19.0">Mihomo 1.19.0</option><option value="mihomo/1.19.17">Mihomo 1.19.17</option><option value="clash-verge/v2.4.5">Clash Verge Rev 2.4.5</option><option value="Stash/3.3.0">Stash 3.3.0</option></select></label>
          <label>输出格式<select value={format} onChange={e => { invalidate(); setFormat(e.target.value); }}><option value="auto">Auto（自动适配）</option><option value="router">完整 YAML</option><option value="clash">Clash YAML</option><option value="shadowrocket">Shadowrocket 完整配置</option><option value="shadowrocket-nodes">Shadowrocket 节点订阅</option></select></label><button className="primary" disabled={busy}>{busy ? "正在模拟…" : "生成预览"}</button></FieldActionRow>
      </form></PanelBody>
    </Panel>
    {result && <Panel title={result.report.client.name ? `${result.report.client.name} ${result.report.client.version ?? "版本未知"}` : "未识别客户端"} description={confidence[result.report.client.confidence] ?? "能力未确认"}>
      <PanelBody><div className="compat-counts" role="status"><span>原始节点 <strong>{result.report.before}</strong></span><span>已排除 <strong>{result.report.removed}</strong></span><span>保留 <strong>{result.report.retained}</strong></span><span>引用修复 <strong>{result.report.repaired_references}</strong></span></div>
        {result.format && <p className="muted">实际输出：{formatNames[result.format] ?? result.format}</p>}
        {result.report.warnings.map(w => <p className="muted" key={w}>{w}</p>)}
        {result.report.blocked_groups.length > 0 && <p className="muted">以下分组已设为拒绝连接，以保留规则并避免绕过原有链路：{result.report.blocked_groups.join("、")}</p>}
        {result.report.unknown_capabilities.length > 0 && <details><summary>尚未确认的能力（{result.report.unknown_capabilities.length}）</summary><p className="muted">这些能力不会触发自动删除。</p><div className="compat-tags">{result.report.unknown_capabilities.map(key => <code key={key}>{key}</code>)}</div></details>}
      </PanelBody>
      {result.report.exclusions.length > 0 && <div className="table-wrap"><table className="resource-table compat-table"><caption className="sr-only">节点排除原因</caption><thead><tr><th>节点</th><th>协议</th><th>原因</th></tr></thead><tbody>{result.report.exclusions.map((node, index) => <tr key={index}><td>{node.name}</td><td><code>{node.protocol}</code></td><td>{reasons[node.reason] ?? node.reason}<small>{node.capability}</small></td></tr>)}</tbody></table></div>}
      {result.report.removed > result.report.exclusions.length && <PanelBody><p className="muted">最多展示前 200 条排除记录。</p></PanelBody>}
    </Panel>}
    <ConfigPreview title="客户端收到的配置" formatLabel={result?.format === "shadowrocket-nodes" ? "Base64 节点订阅" : "YAML"} content={result?.content} error={error || result?.error} loading={busy} empty="输入请求头后生成预览，查看节点和分组的最终输出。" />
  </>;
}

function MatrixPanel({ matrix }: { matrix: Matrix }) {
  const [params, setParams] = useSearchParams();
  const client = matrix.clients.find(v => v.id === params.get("client")) ?? matrix.clients[0];
  const versions = [...client.releases.map(r => ({ label: r.version, profile: r.profile, evidence: r.evidence, prerelease: r.prerelease })), ...client.ranges.map(r => ({ label: `${r.min} – ${r.max}`, profile: r.profile, evidence: "", prerelease: false }))];
  const selected = versions.find(v => v.label === params.get("client_version")) ?? versions.at(-1);
  const profile = selected?.profile ? matrix.profiles[selected.profile] : undefined;
  function choose(key: string, value: string) { const next = new URLSearchParams(params); next.set(key, value); if (key === "client") next.delete("client_version"); setParams(next); }
  const status = (supported?: boolean) => <span className={`status ${supported === true ? "good" : supported === false ? "bad" : "neutral"}`}><i />{supported === true ? "支持" : supported === false ? "不支持" : "未确认"}</span>;
  const evidence = [...new Set([selected?.evidence, ...profile?.evidence ?? []].filter((value): value is string => !!value))];
  return <Panel title="客户端能力矩阵" description={`更新于 ${matrix.checked_at} · ${matrix.clients.length} 个客户端系列 · ${matrix.clients.reduce((n, c) => n + c.releases.length + c.ranges.length, 0)} 条版本记录`}>
    <PanelBody className="compat-form"><FieldActionRow><label>客户端<select value={client.id} onChange={e => choose("client", e.target.value)}>{matrix.clients.map(c => <option key={c.id} value={c.id}>{c.name}</option>)}</select></label><label>版本<select disabled={!versions.length} value={selected?.label ?? ""} onChange={e => choose("client_version", e.target.value)}>{versions.length ? versions.map(v => <option key={v.label} value={v.label}>{v.label}{v.prerelease ? "（预发行）" : ""}{v.profile ? "" : "（未核实）"}</option>) : <option value="">尚无已核实版本映射</option>}</select></label></FieldActionRow>
      <p className="muted">{client.notes}</p><p className="muted">配置类型：{client.format === "clash" ? "Clash YAML" : client.format}。协议能力与输出格式分别判断；此表不保证其他客户端能导入当前导出格式。</p>
      {!profile && <p className="muted">此版本缺少可靠能力证据，自动模式保留节点。可在过滤策略中手动排除类型。</p>}
      {evidence.length > 0 && <details><summary>官方依据（{evidence.length}）</summary><ul className="compat-evidence">{evidence.map((url, i) => <li key={url}><a href={url} target="_blank" rel="noreferrer">依据 {i + 1} · {new URL(url).hostname}</a></li>)}</ul></details>}
    </PanelBody>
    <div className="table-wrap"><table className="resource-table compat-table"><caption className="sr-only">{client.name} {selected?.label} 协议能力</caption><thead><tr><th>协议</th><th>客户端能力</th><th>自动策略</th></tr></thead><tbody>{matrix.protocols.map(type => {
      const supported = profile?.protocols.includes(type) ? true : profile && (profile.complete_protocols || profile.unsupported_protocols?.includes(type)) ? false : undefined;
      return <tr key={type}><td><code>{type}</code></td><td>{status(supported)}</td><td>{supported === false ? "排除" : supported === true ? "继续检查节点功能" : "保留并提示"}</td></tr>;
    })}</tbody></table></div>
    {profile && Object.keys(profile.features).length > 0 && <PanelBody><details><summary>传输与协议功能（{Object.keys(profile.features).length}）</summary><div className="table-wrap"><table className="resource-table compat-table"><thead><tr><th>功能</th><th>客户端能力</th></tr></thead><tbody>{Object.entries(profile.features).map(([key, value]) => <tr key={key}><td><code>{key}</code></td><td>{status(value)}</td></tr>)}</tbody></table></div></details></PanelBody>}
  </Panel>;
}
