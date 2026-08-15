const NOTICE_ZH: Record<string, string> = {
  'core stopped via signal': '内核已停止',
  'core stopped via IPC': '内核已停止',
  started: '已启动',
  stopped: '已停止',
  downloaded: '已下载',
  selected: '已切换',
  success: '完成',
  start_requested: '正在启动',
  stop_requested: '正在停止',
  restart_requested: '正在重启',
  'starting core': '正在启动内核',
  'stopping core': '正在停止内核',
  'restarting core': '正在重启内核',
  'core is not running': '内核未运行',
  'core binary not found': '未找到内核',
}

export function localizeNotice(message: string): string {
  return NOTICE_ZH[message.trim()] ?? message
}

export function formatTime(value?: string | null): string {
  if (!value) return ''
  const numeric = Number(value)
  const date =
    Number.isFinite(numeric) && numeric > 1_000_000_000
      ? new Date(numeric > 1e12 ? numeric : numeric * 1000)
      : new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return date.toLocaleString('zh-CN', { hour12: false })
}
