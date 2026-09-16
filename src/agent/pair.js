const $ = (id) => document.getElementById(id);
let checking = false,
  retrying = false,
  submitting = false,
  available = false;
let selectedAction = null;
const terminal = {
  denied: "你已取消授权。设备尚未绑定。",
  expired: "本次授权已过期，请重新开始。",
  error: "授权未完成，请检查云端连接后重新开始。",
  save_error:
    "云端已授权，但无法保存本地配置。请检查配置文件权限，修复后重新授权。",
};
const actions = { start: "启动", stop: "停止", restart: "重启" };
function message(text, error = false) {
  $("message").hidden = !text;
  $("message").textContent = text;
  $("message").classList.toggle("error", error);
}
function buttons() {
  document.querySelectorAll("[data-action]").forEach((button) => {
    button.disabled = submitting || !available;
  });
}
async function status() {
  if (checking) return;
  checking = true;
  try {
    const response = await fetch("/api/pair/status", { cache: "no-store" });
    if (!response.ok) throw Error();
    const data = await response.json();
    $("version").textContent = data.agent_version
      ? "v" + data.agent_version
      : "";
    available = !!data.bound && !!data.authorized;
    $("binding").hidden = !!data.bound;
    $("dashboard").hidden = !available;
    $("unlock").hidden = !data.bound || !!data.authorized;
    $("proxy-panel").hidden = !available;
    if (available) renderProxies(data.runtime?.proxy_state);
    if (data.bound) {
      $("title").textContent = "设备控制台";
      $("intro").textContent = "配置自动同步，运行由你掌控。";
      message("");
      const runtime = data.runtime || {};
      $("identity-name").textContent = data.identity_name || "等待云端分配";
      $("cloud-name").textContent = data.cloud_url || "—";
      $("revision").textContent = runtime.revision || "等待下发";
      // The public API keeps its short apex URL; management lives on the console.
      const cloud = new URL(data.cloud_url);
      $("manage").href =
        cloud.origin === "https://camofy.app"
          ? "https://cloud.camofy.app/"
          : cloud.href;
      $("core-state").textContent =
        {
          running: "运行中",
          stopped: "已停止",
          unavailable: "未就绪",
          unbound: "等待配置",
        }[runtime.core_state] || "未知状态";
      $("core-state").dataset.state = runtime.core_state || "unknown";
      $("state-description").textContent =
        {
          running: "内核正在运行。启用的代理能力由当前身份配置决定。",
          stopped: "内核已停止，不会接管流量。配置仍可在后台同步。",
          unavailable: "内核暂未就绪，请检查下方错误信息。",
          unbound: "等待云端下发配置后，即可启动内核。",
        }[runtime.core_state] || "暂时无法确定内核运行状态。";
      $("core-error").textContent = runtime.error || "";
      $("core-error").hidden = !runtime.error;
      if (location.pathname === "/bind/complete")
        history.replaceState(null, "", "/");
    } else if (terminal[data.phase] && !retrying) {
      $("form").hidden = true;
      $("pending").hidden = true;
      $("retry").hidden = false;
      message(terminal[data.phase], true);
    } else if (data.phase === "pending" || data.phase === "starting") {
      $("form").hidden = true;
      message("等待云端授权，完成后自动返回设备控制台。");
    } else if ($("message").classList.contains("error") && !$("form").hidden) {
      // Keep authorization errors visible until the user retries.
    } else if (!retrying) message("");
  } catch {
    available = false;
    $("core-state").textContent = "连接中断";
    $("core-state").dataset.state = "unknown";
    message("暂时无法连接本机 Agent，正在重试…", true);
  } finally {
    checking = false;
    buttons();
  }
}
function headers() {
  return {
    "Content-Type": "application/json",
    "X-Camofy-CSRF": document.querySelector('meta[name="csrf"]').content,
  };
}
$("form").addEventListener("submit", async (event) => {
  event.preventDefault();
  $("start").disabled = true;
  message("");
  try {
    const response = await fetch("/api/pair/start", {
      method: "POST",
      headers: headers(),
      body: JSON.stringify({
        device_name: $("name").value,
        cloud_url: $("cloud").value,
      }),
    });
    const data = await response.json();
    if (!response.ok) throw Error(data.error || "无法开始授权");
    retrying = false;
    $("user-code").textContent = data.user_code;
    $("authorize").href = data.verification_uri;
    $("pending").hidden = false;
    $("form").hidden = true;
    location.assign(data.verification_uri);
  } catch (error) {
    message(error.message, true);
  } finally {
    $("start").disabled = false;
  }
});
$("retry").addEventListener("click", () => {
  retrying = true;
  $("form").hidden = false;
  $("retry").hidden = true;
  message("");
});
$("controls").addEventListener("click", (event) => {
  const button = event.target.closest("button[data-action]");
  if (!button || button.disabled || submitting) return;
  selectedAction = button.dataset.action;
  $("confirm-title").textContent = actions[selectedAction] + " Mihomo？";
  $("confirm-description").textContent =
    selectedAction === "stop"
      ? "停止代理内核可能中断现有连接。配置同步不会将它自动启动。"
      : "将按当前配置运行内核。如果配置启用了 TUN，可能接管设备流量。";
  $("confirm-action").textContent = "确认" + actions[selectedAction];
  $("confirm-dialog").returnValue = "cancel";
  $("confirm-dialog").showModal();
});
$("confirm-dialog").addEventListener("close", async () => {
  if (
    $("confirm-dialog").returnValue !== "confirm" ||
    !selectedAction ||
    submitting ||
    !available
  )
    return;
  submitting = true;
  buttons();
  $("control-result").hidden = false;
  $("control-result").classList.remove("error");
  $("control-result").textContent = "正在提交操作…";
  try {
    const response = await fetch("/api/core/control", {
      method: "POST",
      headers: headers(),
      body: JSON.stringify({ action: selectedAction }),
    });
    if (!response.ok) throw Error((await response.json()).error || "操作失败");
    $("control-result").textContent = "操作已排队，请以上方内核状态为准。";
  } catch (error) {
    $("control-result").textContent = error.message;
    $("control-result").classList.add("error");
  } finally {
    selectedAction = null;
    submitting = false;
    buttons();
    void status();
  }
});
void status();
setInterval(status, 2500);
async function localApi(path,body){
  const r=await fetch(path,{method:"POST",headers:{"Content-Type":"application/json","X-Camofy-CSRF":document.querySelector('meta[name="csrf"]').content},body:JSON.stringify(body)});
  if(!r.ok)throw Error((await r.json()).error||"操作失败");
  return r.status===204?null:r.json();
}
$("unlock-form").addEventListener("submit",async e=>{
  e.preventDefault();try{await localApi("/api/local/login",{key:$("admin-key").value});$("admin-key").value="";$("unlock-error").textContent="";void status();}catch(e){$("unlock-error").textContent=e.message;}
});
let proxyState=null, proxyFingerprint="", proxyBusy=false;
const delays=new Map();
function renderProxies(state){
  proxyState=state;
  if(proxyBusy)return;
  const query=$("proxy-search").value.toLowerCase();
  const fingerprint=JSON.stringify([state?.groups,state?.desired,state?.overrides,state?.errors,state?.status,state?.pending_local,query,[...delays]]);
  if(fingerprint===proxyFingerprint)return;proxyFingerprint=fingerprint;
  const container=$("proxy-groups");container.replaceChildren();
  if(!state?.groups?.length){container.textContent=state?.status==="stopped"?"内核已停止，启动后可以查询和选择节点。":"等待内核分组信息…";return;}
  for(const group of state.groups){
    const members=group.name.toLowerCase().includes(query)?group.members:group.members.filter(n=>n.toLowerCase().includes(query));if(!members.length)continue;
    const section=document.createElement("section");section.className="proxy-group";
    const title=document.createElement("h3");title.textContent=group.name;section.append(title);
    const description=document.createElement("p");description.textContent=`实际：${group.now||"未知"} · 期望：${state.desired?.[group.name]||"默认"} · ${state.overrides?.[group.name]?"设备覆盖":"跟随身份"}${state.pending_local?" · 本地修改待云端同步":""}`;section.append(description);
    if(state.errors?.[group.name]){const error=document.createElement("p");error.textContent=`尚未确认：${state.errors[group.name]}`;error.className="error";section.append(error);}
    if(state.overrides?.[group.name]){const reset=document.createElement("button");reset.className="secondary reset";reset.textContent="恢复跟随身份";reset.onclick=()=>selectLocal(group.name,null);section.append(reset);}
    const grid=document.createElement("div");grid.className="proxy-grid";
    for(const name of members){
      const node=document.createElement("div");node.className=`proxy-node ${group.now===name?"selected":""}`;
      const select=document.createElement("button");select.textContent=name;select.disabled=group.kind!=="Selector";select.setAttribute("aria-pressed",String(group.now===name));select.onclick=()=>selectLocal(group.name,name);node.append(select);
      const delay=document.createElement("button");delay.className="delay";delay.textContent=delays.has(name)?`${delays.get(name)} ms`:"测速";
      delay.onclick=async()=>{delay.disabled=true;try{const value=await localApi("/api/proxies",{method:"proxies.delay",params:{name}});delays.set(name,value.delay??"超时");proxyFingerprint="";renderProxies(proxyState);}catch(e){$("proxy-feedback").textContent=e.message;}finally{delay.disabled=false;}};node.append(delay);grid.append(node);
    }
    section.append(grid);container.append(section);
  }
}
async function selectLocal(group,node){
  if(proxyBusy)return;proxyBusy=true;$("proxy-feedback").textContent="正在保存并确认选择…";
  try{const state=await localApi("/api/proxies",{method:"proxies.select",params:{group,node}});proxyState=state;$("proxy-feedback").textContent=state.status==="applied"?"已回读确认。设备覆盖将自动同步到云端。":"期望选择已保存，尚未全部应用；请查看分组反馈。";}catch(e){$("proxy-feedback").textContent=e.message;}finally{proxyBusy=false;proxyFingerprint="";renderProxies(proxyState);}
}
$("proxy-search").addEventListener("input",()=>renderProxies(proxyState));
