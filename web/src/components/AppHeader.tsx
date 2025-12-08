type AppHeaderProps = {
  passwordSet: boolean
  authToken: string | null
  onLogout: () => void
}

function AppHeader({ passwordSet, authToken, onLogout }: AppHeaderProps) {
  return (
    <header className="mb-6 border-b border-slate-800 pb-4">
      <h1 className="text-2xl font-semibold tracking-tight">Camofy</h1>
      <p className="mt-1 text-sm text-slate-400">
        路由器上的 Mihomo Web 管理面板（已实现里程碑 1–5：基础 HTTP 服务、订阅管理、内核下载与运行控制、用户配置合并与安全监控）
      </p>
      {passwordSet && authToken && (
        <div className="mt-2 flex items-center justify-between text-xs text-slate-400">
          <span>已登录</span>
          <button
            type="button"
            className="rounded border border-slate-700 bg-slate-900 px-2 py-0.5 text-[11px] text-slate-200 hover:bg-slate-800"
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

