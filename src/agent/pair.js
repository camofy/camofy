const $ = (id) => document.getElementById(id);
let checking = false,
  retrying = false;
const terminal = {
  denied: "你已取消授权。设备尚未绑定。",
  expired: "本次授权已过期，请重新开始。",
  error: "授权未完成，请检查云端连接后重新开始。",
  save_error:
    "云端已授权，但无法保存本地配置。请检查配置文件权限，修复后重新授权。",
};
function message(text, error = false) {
  $("message").hidden = false;
  $("message").textContent = text;
  $("message").classList.toggle("error", error);
}
async function status() {
  if (checking) return;
  checking = true;
  try {
    const response = await fetch("/api/pair/status", { cache: "no-store" });
    if (!response.ok) throw Error();
    const data = await response.json();
    if (data.bound) {
      $("title").textContent = "设备控制台";
      $("intro").textContent =
        "身份由云端分配，配置自动下发。本地控制在云端断开时仍可使用。";
      $("form").hidden = true;
      $("pending").hidden = true;
      $("retry").hidden = true;
      message(
        data.identity_name
          ? `绑定身份：${data.identity_name}`
          : "已绑定，等待云端下发身份。",
      );
      $("manage").href = data.cloud_url;
      $("manage").hidden = false;
      $("controls").hidden = !data.authorized;
      $("unlock").hidden = data.authorized;
      $("proxy-panel").hidden = !data.authorized;
      if (data.authorized) renderProxies(data.runtime.proxy_state);
      $("core-state").textContent =
        {
          running: "运行中",
          stopped: "已停止",
          unavailable: "未就绪",
          unbound: "等待配置",
        }[data.runtime.core_state] || "读取状态中";
      $("core-error").textContent = data.runtime.error || "";
      if (location.pathname === "/bind/complete")
        history.replaceState(null, "", "/");
    } else if (terminal[data.phase] && !retrying) {
      $("form").hidden = true;
      $("pending").hidden = true;
      $("retry").hidden = false;
      message(terminal[data.phase], true);
    } else if (data.phase === "pending" || data.phase === "starting") {
      $("form").hidden = true;
      message("等待云端授权…完成后将在此显示绑定结果。");
    }
  } catch {
    message("暂时无法连接本机 Agent，正在重试…", true);
  } finally {
    checking = false;
  }
}
$("form").addEventListener("submit", async (e) => {
  e.preventDefault();
  $("start").disabled = true;
  $("message").hidden = true;
  try {
    const response = await fetch("/api/pair/start", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-Camofy-CSRF": document.querySelector('meta[name="csrf"]').content,
      },
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
  $("message").hidden = true;
});
const controls = document.createElement("section");
controls.id = "controls";
controls.hidden = true;
controls.innerHTML =
  '<h2>Mihomo <small id="core-state"></small></h2><p>停止后不会被自动同步重新启动。启停可能改变网络连接。</p><div class="core-actions"><button data-action="start">启动</button><button data-action="stop">停止</button><button data-action="restart">重启</button></div><p id="core-error" role="alert"></p><p id="control-result" role="status"></p>';
$("manage").before(controls);
controls.addEventListener("click", async (e) => {
  const button = e.target.closest("button[data-action]");
  if (!button) return;
  if (!confirm(`确定${button.textContent} Mihomo？这可能影响设备网络。`))
    return;
  button.disabled = true;
  try {
    const r = await fetch("/api/core/control", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-Camofy-CSRF": document.querySelector('meta[name="csrf"]').content,
      },
      body: JSON.stringify({ action: button.dataset.action }),
    });
    if (!r.ok) throw Error((await r.json()).error || "操作失败");
    $("control-result").textContent =
      "操作已排队，完成当前配置操作后执行。请查看上方实时状态。";
  } catch (error) {
    $("control-result").textContent = error.message;
  } finally {
    button.disabled = false;
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
  const fingerprint=JSON.stringify([state?.groups,state?.desired,state?.errors,state?.status,state?.pending_local,query,[...delays]]);
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
