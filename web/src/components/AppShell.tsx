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
    <div className="app-root bg-[color:var(--color-bg-app)] text-[color:var(--color-text-main)]">
      <div className="mx-auto flex h-screen max-w-5xl flex-col px-4 py-8">
        <AppHeader passwordSet={passwordSet} authToken={token} onLogout={logout} />

        <NotificationBar />

        <div className="mt-4 flex flex-1 min-h-0 flex-col gap-4 overflow-hidden">
          {children}
        </div>

        <footer className="mt-6 border-t border-[color:var(--color-border-subtle)] pt-4">
          <p className="text-xs text-[color:var(--color-text-soft)]">
            当前进度：已完成里程碑 1–5（HTTP 服务与静态页面、订阅管理、内核下载与运行控制、用户
            profile 管理与配置合并、面板访问密码与日志监控）。
          </p>
        </footer>
      </div>
    </div>
  )
}

export default AppShell
