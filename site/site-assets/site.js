"use strict";
const copyButton = document.getElementById("copy-command");
copyButton.addEventListener("click", async () => {
  const status = document.getElementById("copy-status");
  try {
    await navigator.clipboard.writeText(
      document.getElementById("install-command").textContent.trim(),
    );
    copyButton.textContent = "已复制";
    status.textContent = "已复制。请确认设备安装要求后执行。";
  } catch {
    status.textContent = "未能复制，请选中上方命令手动复制。";
  }
  setTimeout(() => {
    copyButton.textContent = "复制";
  }, 2400);
});
