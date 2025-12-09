import type { ReactNode } from 'react'
import { useAuth } from '../context/AuthContext'
import AppHeader from './AppHeader'
import NotificationBar from './NotificationBar'

type AppShellProps = {
  children: ReactNode
}

function AppShell({ children }: AppShellProps) {
  const { passwordSet, token, logout } = useAuth()

  return (
    <div className="app-root bg-slate-950 text-slate-100">
      <div className="mx-auto flex min-h-screen max-w-5xl flex-col px-4 py-8">
        <AppHeader passwordSet={passwordSet} authToken={token} onLogout={logout} />

        <NotificationBar />

        {children}

        <footer className="mt-6 border-t border-slate-800 pt-4">
          <p className="text-xs text-slate-500">
            当前进度：已完成里程碑 1–5（HTTP 服务与静态页面、订阅管理、内核下载与运行控制、用户
            profile 管理与配置合并、面板访问密码与日志监控）。
          </p>
        </footer>
      </div>
    </div>
  )
}

export default AppShell

