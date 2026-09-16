"use strict";
const devices = {
  desktop: { label: "DESKTOP / 电脑", glyph: "⌘", title: "在 Clash Verge Rev 中，继续你的习惯。", description: "复制身份的订阅链接，导入客户端。节点、策略组和规则来自同一份合并配置。", labelText: "身份订阅链接", value: "https://camofy.app/sub/…", status: "在客户端刷新订阅后生效", detail: "你的端口、系统代理等设置仍由客户端管理。", action: "创建一个身份 ↗", href: "https://cloud.camofy.app/identities" },
  mobile: { label: "MOBILE / 手机", glyph: "◫", title: "走到哪里，带上熟悉的网络配置。", description: "在身份详情中选择 Shadowrocket 输出，复制对应链接到手机，获取节点订阅或包含规则的配置。", labelText: "两种输出，按需选择", value: "节点订阅 / 完整配置", status: "在手机客户端中导入并刷新", detail: "不同客户端的协议和规则支持存在差异，以云端输出提示为准。", action: "管理订阅与身份 ↗", href: "https://cloud.camofy.app/identities" },
  router: { label: "ROUTER / 路由器", glyph: "⌁", title: "轻量 Agent，让路由器专注运行。", description: "安装后打开设备本地页面，跳转云端登录授权。给设备关联身份，配置由云端下发，无需手填订阅链接。", labelText: "设备绑定流程", value: "本地授权 → 云端绑定 → 拉取配置", status: "实时通知 + 五分钟轮询兜底", detail: "首次安装时 Mihomo 保持停止；确认配置后再启动。", action: "查看 Agent 安装方式 ↓", href: "#install" },
};
const tabs = [...document.querySelectorAll("[data-device]")];
function selectDevice(tab) {
  const data = devices[tab.dataset.device];
  tabs.forEach((item) => { const selected = item === tab; item.setAttribute("aria-selected", String(selected)); item.tabIndex = selected ? 0 : -1; });
  document.getElementById("device-panel").setAttribute("aria-labelledby", tab.id);
  for (const [id, key] of Object.entries({"device-label":"label", "device-glyph":"glyph", "device-title":"title", "device-description":"description", "demo-label":"labelText", "demo-value":"value", "demo-status":"status", "device-detail":"detail", "device-action":"action"})) document.getElementById(id).textContent = data[key];
  document.getElementById("device-action").href = data.href;
}
tabs.forEach((tab, index) => {
  tab.addEventListener("click", () => selectDevice(tab));
  tab.addEventListener("keydown", (event) => {
    let next;
    if (["ArrowRight", "ArrowDown"].includes(event.key)) next = (index + 1) % tabs.length;
    else if (["ArrowLeft", "ArrowUp"].includes(event.key)) next = (index + tabs.length - 1) % tabs.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = tabs.length - 1;
    else return;
    event.preventDefault(); selectDevice(tabs[next]); tabs[next].focus();
  });
});
document.getElementById("copy-command").addEventListener("click", async () => {
  const status = document.getElementById("copy-status");
  try { await navigator.clipboard.writeText(document.getElementById("install-command").textContent.trim()); status.textContent = "安装命令已复制。执行前请确认设备架构和安装目录。"; }
  catch { status.textContent = "浏览器未允许复制，请选中上方命令手动复制。"; }
});
