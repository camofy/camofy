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
      $("controls").hidden = false;
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
