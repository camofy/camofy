import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from 'react'
import type { Settings } from '../types'
import type { AuthedFetch } from '../api'
import { fetchSettings, login as apiLogin } from '../api'

type AuthContextValue = {
  token: string | null
  authReady: boolean
  passwordSet: boolean
  authedFetch: AuthedFetch
  settings: Settings | null
  refreshSettings: () => Promise<void>
  login: (password: string) => Promise<void>
  logout: () => void
}

const AuthContext = createContext<AuthContextValue | undefined>(undefined)

export function AuthProvider({ children }: { children: ReactNode }) {
  const [token, setToken] = useState<string | null>(null)
  const [authReady, setAuthReady] = useState(false)
  const [passwordSet, setPasswordSet] = useState(false)
  const [settings, setSettings] = useState<Settings | null>(null)

  const authedFetch: AuthedFetch = useCallback(
    (input: RequestInfo | URL, init?: RequestInit) => {
      const headers = new Headers(init?.headers ?? {})
      if (token) {
        headers.set('X-Auth-Token', token)
      }
      return fetch(input, { ...init, headers })
    },
    [token],
  )

  const loadSettings = useCallback(
    async (initialToken: string | null) => {
      try {
        const fetchWithToken: AuthedFetch = (input, init) => {
          const headers = new Headers(init?.headers ?? {})
          if (initialToken) {
            headers.set('X-Auth-Token', initialToken)
          }
          return fetch(input, { ...init, headers })
        }
        const body = await fetchSettings(fetchWithToken)
        if (body.code === 'unauthorized') {
          setPasswordSet(true)
          setSettings(null)
          setToken(null)
          localStorage.removeItem('freeNetworkToken')
        } else if (body.code === 'ok' && body.data) {
          setPasswordSet(body.data.password_set)
          setSettings(body.data)
        } else if (body.message) {
          // 仅记录日志，由调用方决定是否提示
          console.error(body.message)
        }
      } catch (err) {
        console.error('failed to load settings', err)
      } finally {
        setAuthReady(true)
      }
    },
    [],
  )

  useEffect(() => {
    const storedToken = localStorage.getItem('freeNetworkToken')
    if (storedToken) {
      setToken(storedToken)
    }
    void loadSettings(storedToken)
  }, [loadSettings])

  const refreshSettings = useCallback(async () => {
    await loadSettings(token)
  }, [loadSettings, token])

  const login = useCallback(
    async (password: string) => {
      const trimmed = password.trim()
      if (!trimmed) {
        throw new Error('请输入面板访问密码')
      }

      const body = await apiLogin(trimmed)
      if (body.code !== 'ok' || !body.data) {
        throw new Error(body.message || '登录失败')
      }

      setToken(body.data.token)
      localStorage.setItem('freeNetworkToken', body.data.token)
      await refreshSettings()
    },
    [refreshSettings],
  )

  const logout = useCallback(() => {
    setToken(null)
    localStorage.removeItem('freeNetworkToken')
  }, [])

  const value: AuthContextValue = useMemo(
    () => ({
      token,
      authReady,
      passwordSet,
      authedFetch,
      settings,
      refreshSettings,
      login,
      logout,
    }),
    [token, authReady, passwordSet, authedFetch, settings, refreshSettings, login, logout],
  )

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>
}

export function useAuth(): AuthContextValue {
  const ctx = useContext(AuthContext)
  if (!ctx) {
    throw new Error('useAuth must be used within AuthProvider')
  }
  return ctx
}

