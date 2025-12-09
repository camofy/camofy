import { type FormEvent, useState } from 'react'
import { useLocation, useNavigate } from 'react-router-dom'
import { useAuth } from '../context/AuthContext'
import { useNotifications } from '../context/NotificationContext'
import AppShell from '../components/AppShell'
import LoginPanel from '../components/LoginPanel'

function LoginPage() {
  const { login, authReady, passwordSet, token } = useAuth()
  const { notifyError, notifySuccess } = useNotifications()
  const [password, setPassword] = useState('')
  const [loading, setLoading] = useState(false)
  const navigate = useNavigate()
  const location = useLocation() as { state?: { from?: Location } }

  const fromPath =
    location.state?.from?.pathname && location.state.from.pathname !== '/login'
      ? location.state.from.pathname
      : '/overview'

  if (authReady && passwordSet && token) {
    navigate(fromPath, { replace: true })
  }

  const handleSubmit = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault()
    setLoading(true)
    try {
      await login(password)
      setPassword('')
      notifySuccess('登录成功')
      navigate(fromPath, { replace: true })
    } catch (err) {
      const msg = err instanceof Error ? err.message : '登录失败'
      notifyError(msg)
    } finally {
      setLoading(false)
    }
  }

  return (
    <AppShell>
      <main className="flex flex-1 items-center justify-center">
        <LoginPanel
          loginPassword={password}
          onLoginPasswordChange={setPassword}
          onSubmit={handleSubmit}
          loading={loading}
        />
      </main>
    </AppShell>
  )
}

export default LoginPage

