import { useState, type ReactNode } from 'react'
import { useLocation } from 'react-router-dom'
import { useAuth } from '../context/AuthContext'
import NotificationBar from './NotificationBar'
import Sidebar from './Sidebar'
import Topbar from './Topbar'

type AppShellProps = {
  children: ReactNode
}

function AppShell({ children }: AppShellProps) {
  const { passwordSet, token, logout } = useAuth()
  const location = useLocation()
  const [collapsed, setCollapsed] = useState(false)
  const [mobileOpen, setMobileOpen] = useState(false)

  return (
    <div className="app-root flex h-full overflow-hidden text-[color:var(--color-text-main)]">
      <Sidebar
        collapsed={collapsed}
        mobileOpen={mobileOpen}
        onToggleCollapsed={() => setCollapsed((value) => !value)}
        onCloseMobile={() => setMobileOpen(false)}
      />

      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <Topbar
          pathname={location.pathname}
          passwordSet={passwordSet}
          authToken={token}
          onLogout={logout}
          onOpenMobile={() => setMobileOpen(true)}
        />
        <NotificationBar />
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden px-6 pb-6">
          {children}
        </div>
      </div>
    </div>
  )
}

export default AppShell
