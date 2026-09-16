import { useCallback, useEffect, useState } from "react";
import { api, displayTime, type Resource } from "./model";
import { Panel, PanelBody, FieldActionRow, Icon } from "./ui";
import "./proxy-control.css";

type Group = { name: string; kind: string; members: string[]; now?: string; dynamic: boolean };
type State = { groups?: Group[]; status?: string; errors?: Record<string,string>; received_at?: number; sampled_at?: number; pending_local?: boolean; desired?: Record<string,string>; overrides?: Record<string,string>; selection_version?: number; override_version?: number };
type Job = { id: string; method: string; status: string; expires_at: number; result?: { error?: string; value?: { name?: string; delay?: number; sampled_at?: number } } };
type View = { groups?: Group[]; selections?: Record<string,string>; overrides?: Record<string,string>; version: number; state?: State; reported?: { protocol?: number; seen_at?: number; core_state?: string }; jobs?: Job[]; devices?: { id: string; name: string; reported?: { seen_at?: number; proxy_state?: State }; overrides?: Record<string,string> }[] };
const labels: Record<string,string> = { queued:"等待设备", executing:"执行中", succeeded:"已确认", failed:"失败", expired:"已过期", superseded:"已被新操作替代", unknown:"结果未知", applied:"已应用", partial:"部分未应用", stopped:"内核已停止", unavailable:"内核未就绪" };
const errorLabels: Record<string,string> = { group_missing:"分组已不存在", node_missing:"节点未找到或 provider 尚未加载", not_selectable:"该分组不支持手动选择", apply_failed:"切换失败，正在重试", readback_mismatch:"尚未确认实际选择" };

export function ProxyControl({ r }: { r: Resource }) {
  const [view,setView]=useState<View>();
  const [query,setQuery]=useState("");
  const [error,setError]=useState("");
  const [message,setMessage]=useState("");
  const [busy,setBusy]=useState(false);
  const device=r.kind==="device";
  const load=useCallback(async()=>{try{setView(await api<View>(`/resources/${r.id}/proxies`));setError("");}catch(e){setError((e as Error).message);}},[r.id]);
  useEffect(()=>{void load();const timer=setInterval(()=>{if(!document.hidden)void load();},5000);return()=>clearInterval(timer);},[load]);
  async function change(group:string,node?:string){
    if(!view)return;setBusy(true);setMessage("");
    const selections={...(device?view.overrides:view.selections)};
    if(node===undefined)delete selections[group];else selections[group]=node;
    try{await api(`/resources/${r.id}/selections`,"PUT",{expected_version:view.version,selections});setMessage(device?"设备期望选择已保存，等待实际回读确认。":"身份选择已保存，绑定设备将同步；第三方客户端需更新订阅，且可能保留本地选择。");await load();}
    catch(e){setError((e as Error).message);}finally{setBusy(false);}
  }
  async function rpc(method:string,params:unknown=null){
    setBusy(true);try{await api(`/devices/${r.id}/rpc`,"POST",{method,params,idempotency_key:crypto.randomUUID()});setMessage("任务已提交，结果以设备回执为准。");await load();}catch(e){setError((e as Error).message);}finally{setBusy(false);}
  }
  const stale=device&&(!view?.state?.received_at||Date.now()/1000-view.state.received_at>360);
  return <div className="proxy-control">
    <Panel title={device?"设备代理":"身份共享选择"} actions={<button disabled={busy} onClick={()=>device?void rpc("proxies.list"):void load()}><Icon name="refresh" size={15}/>刷新状态</button>}>
      <PanelBody>
        <p className="muted">{device?"这里的切换仅覆盖当前设备。恢复跟随身份后，使用共享选择。":"在身份内选择，所有跟随身份的 Agent 自动同步。自动测速组可以拥有不同的实际出口。"} 切换不强制断开已有连接。</p>
        {device&&<p className="proxy-meta">{stale?"离线或状态已过期 · 以下为最后快照":labels[view?.state?.status??""]??"等待设备快照"} · 采集于 {displayTime(view?.state?.sampled_at)}{view?.state?.pending_local?" · 设备有离线本地覆盖待同步":""}</p>}
        <FieldActionRow><label className="proxy-search">搜索分组或节点<input value={query} onChange={e=>setQuery(e.target.value)} placeholder="输入名称筛选"/></label></FieldActionRow>
        {device&&<div className="proxy-toolbar">{([['core.start','启动'],['core.stop','停止'],['core.restart','重启']] as const).map(([method,label])=><button key={method} disabled={busy||view?.reported?.protocol!==2} onClick={()=>{if(confirm(`${label}此设备的 Mihomo？可能影响网络，可从本地控制台恢复。`))void rpc(method);}}>{label} Mihomo</button>)}</div>}
        {message&&<p role="status">{message}</p>}
        {error&&<p className="error-text" role="alert">{error}</p>}
      </PanelBody>
    </Panel>
    {!view&&!error&&<p role="status">正在读取分组…</p>}
    {view&&!(view.groups?.length)&&<Panel title="暂无分组快照"><PanelBody><p className="muted">{device?"请确认 Agent 已升级、内核运行中，然后刷新状态。离线时仍可在设备本地操作。":"当前身份没有可显示的代理组。请先配置并发布身份。"}</p></PanelBody></Panel>}
    {view?.groups?.filter(g=>!query||g.name.toLowerCase().includes(query.toLowerCase())||g.members.some(n=>n.toLowerCase().includes(query.toLowerCase()))).map(g=>{
      const desired=(device?view.overrides?.[g.name]:undefined)??view.selections?.[g.name];
      const actual=g.now;
      const selectable=g.kind==="Selector"||g.kind==="select";
      const members=g.name.toLowerCase().includes(query.toLowerCase())?g.members:g.members.filter(n=>n.toLowerCase().includes(query.toLowerCase()));
      return <Panel key={g.name} title={g.name} actions={<span className="chip">{selectable?"手动选择":g.kind} · {g.members.length}</span>}>
        <PanelBody>
          <div className="proxy-group-summary"><div><span className="muted">期望 </span><strong>{desired??"未指定 · 使用客户端默认"}</strong>{device&&<p><span className="muted">实际 </span>{actual??"未知"} {view.overrides?.[g.name]?"· 设备覆盖":"· 跟随身份"}</p>}</div>{(device?view.overrides?.[g.name]:view.selections?.[g.name])&&<button className="quiet" disabled={busy} onClick={()=>void change(g.name)}>{device?"恢复跟随身份":"清除共享选择"}</button>}</div>
          {view.state?.errors?.[g.name]&&<p role="alert" className="error-text">{errorLabels[view.state.errors[g.name]]??view.state.errors[g.name]}</p>}
          {desired&&!g.members.includes(desired)&&<p className="error-text">当前成员中没有期望节点；不会自动改选其他同名近似节点。</p>}
          {g.dynamic&&<p className="muted">包含动态 provider；成员参考设备快照，不代表每台设备已加载完成。</p>}
          <div className="proxy-node-grid">{members.map(node=>{
            const delay=view.jobs?.slice().reverse().find(j=>j.method==="proxies.delay"&&j.status==="succeeded"&&j.result?.value?.name===node)?.result?.value;
            return <div className={`proxy-node ${desired===node?"is-desired":""}`} key={node}><button className="proxy-node-select" aria-pressed={desired===node} disabled={busy||!selectable} onClick={()=>void change(g.name,node)}><span>{node}</span><small>{device&&actual===node?"实际选中":desired===node?"期望选中":""}</small></button>{device&&<button className="proxy-delay" disabled={busy||stale||view.reported?.protocol!==2} title={delay?`设备测速 ${displayTime(delay.sampled_at)}`:"从这台设备测速"} onClick={()=>void rpc("proxies.delay",{name:node})}>{delay?.delay===undefined?"测速":`${delay.delay} ms`}</button>}</div>;
          })}</div>
          {!selectable&&<p className="muted">自动策略只读，不将手动点击转换成固定节点。</p>}
        </PanelBody>
      </Panel>;
    })}
    {!device&&view?.devices&&<Panel title="绑定设备同步情况"><PanelBody><div className="proxy-device-list">{view.devices.length?view.devices.map(d=>{
      const s=d.reported?.proxy_state;
      const offline=!s?.received_at||Date.now()/1000-s.received_at>360;
      const current=s?.selection_version===view.version;
      return <div key={d.id}><a href={`/devices/${d.id}?tab=proxies`}>{d.name}</a><span>{offline?"离线 / 状态过期":!current?"等待同步":labels[s?.status??""]??"等待确认"}{Object.keys(d.overrides??{}).length?" · 有设备覆盖":""}</span></div>;
    }):<p className="muted">没有绑定设备。通过订阅 URL 导入的第三方客户端不提供状态回执。</p>}</div></PanelBody></Panel>}
    {device&&!!view?.jobs?.length&&<Panel title="设备操作回执"><PanelBody><div className="proxy-job-list">{view.jobs.slice(-12).reverse().map(j=><div key={j.id}><strong>{j.method}</strong><span>{labels[j.status]??j.status}</span><small>{j.result?.error??(j.method==="proxies.delay"&&j.result?.value?`${j.result.value.name} · ${j.result.value.delay??"超时"} ms`:"")}</small></div>)}</div></PanelBody></Panel>}
  </div>;
}
