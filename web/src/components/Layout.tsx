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
        ? 'border-sky-500/70 bg-slate-800 text-sky-300'
        : 'border-transparent text-slate-300 hover:text-sky-300 hover:bg-slate-900',
    ].join(' ')

  return (
    <AppShell>
      <nav className="mb-4 flex flex-wrap items-center gap-2 border-b border-slate-800 pb-2 text-xs">
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

      <main className="flex flex-1 flex-col gap-4">
        <Outlet />
      </main>
    </AppShell>
  )
}

export default Layout

