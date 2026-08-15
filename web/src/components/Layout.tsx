import { Outlet, Navigate, useLocation } from 'react-router-dom'
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

  return (
    <AppShell>
      <main className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto">
        <Outlet />
      </main>
    </AppShell>
  )
}

export default Layout
