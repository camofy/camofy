import { NavLink, Outlet, Navigate, useLocation } from 'react-router-dom'
import AppShell from './AppShell'
import { useAuth } from '../context/AuthContext'

function Layout() {
  const { authReady, passwordSet, token } = useAuth()
  const location = useLocation()

  if (!authReady) {
    return null
  }

  if (passwordSet && !token) {
    return (
      <Navigate
        to="/login"
        replace
        state={{ from: location }}
      />
    )
  }

  const navLinkClass = ({ isActive }: { isActive: boolean }) =>
    [
      'rounded-md px-3 py-1 text-xs font-medium transition-colors border',
      isActive
        ? 'border-[color:var(--color-border-strong)] bg-[color:var(--color-primary)] text-[color:var(--color-primary-on)]'
        : 'border-transparent text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text-main)] hover:bg-[color:var(--color-surface-soft)]',
    ].join(' ')

  return (
    <AppShell>
      <nav className="mb-4 flex flex-wrap items-center gap-2 border-b border-[color:var(--color-border-subtle)] pb-2 text-xs">
        <NavLink to="/overview" className={navLinkClass}>
          总览
        </NavLink>
        <NavLink to="/subscriptions" className={navLinkClass}>
          订阅管理
        </NavLink>
        <NavLink to="/profiles" className={navLinkClass}>
          用户配置
        </NavLink>
        <NavLink to="/core" className={navLinkClass}>
          内核管理
        </NavLink>
        <NavLink to="/proxies" className={navLinkClass}>
          代理组与节点
        </NavLink>
        <NavLink to="/logs" className={navLinkClass}>
          日志
        </NavLink>
      </nav>

      <main className="flex flex-1 flex-col gap-4 min-h-0">
        <Outlet />
      </main>
    </AppShell>
  )
}

export default Layout
