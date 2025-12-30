type AppHeaderProps = {
  passwordSet: boolean
  authToken: string | null
  onLogout: () => void
}

function AppHeader({ passwordSet, authToken, onLogout }: AppHeaderProps) {
  return (
    <header className="mb-6 border-b border-[color:var(--color-border-subtle)] pb-4">
      <h1 className="text-2xl font-semibold tracking-tight text-[color:var(--color-text-main)]">Camofy</h1>
      <p className="mt-1 text-sm text-[color:var(--color-text-muted)]">
        路由器上的 Mihomo Web 管理面板（已实现里程碑 1–5：基础 HTTP 服务、订阅管理、内核下载与运行控制、用户配置合并与安全监控）
      </p>
      {passwordSet && authToken && (
        <div className="mt-2 flex items-center justify-between text-xs text-[color:var(--color-text-soft)]">
          <span>已登录</span>
          <button
            type="button"
            className="rounded border border-[color:var(--color-border-strong)] bg-[color:var(--color-surface-soft)] px-2 py-0.5 text-[11px] text-[color:var(--color-text-main)] hover:bg-[color:var(--color-accent)]"
            onClick={onLogout}
          >
            退出登录
          </button>
        </div>
      )}
    </header>
  )
}

export default AppHeader
