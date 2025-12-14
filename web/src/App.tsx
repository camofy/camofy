import { Navigate, Route, Routes } from 'react-router-dom'
import { useAuth } from './context/AuthContext'
import Layout from './components/Layout'
import OverviewPage from './pages/OverviewPage'
import SubscriptionsPage from './pages/SubscriptionsPage'
import ProfilesPage from './pages/ProfilesPage'
import CorePage from './pages/CorePage'
import ProxiesPage from './pages/ProxiesPage'
import LogsPage from './pages/LogsPage'
import LoginPage from './pages/LoginPage'

function App() {
  const { authReady } = useAuth()

  if (!authReady) {
    return (
      <div className="app-root bg-slate-950 text-slate-100">
        <div className="mx-auto flex h-screen max-w-5xl flex-col px-4 py-8">
          <p className="text-xs text-slate-400">正在初始化面板…</p>
        </div>
      </div>
    )
  }

  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      <Route element={<Layout />}>
        <Route path="/" element={<Navigate to="/overview" replace />} />
        <Route path="/overview" element={<OverviewPage />} />
        <Route path="/subscriptions" element={<SubscriptionsPage />} />
        <Route path="/profiles" element={<ProfilesPage />} />
        <Route path="/core" element={<CorePage />} />
        <Route path="/proxies" element={<ProxiesPage />} />
        <Route path="/logs" element={<LogsPage />} />
      </Route>
      <Route path="*" element={<Navigate to="/overview" replace />} />
    </Routes>
  )
}

export default App

