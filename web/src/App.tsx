import { type FormEvent, useEffect, useState } from 'react'
import './App.css'
import type {
  ApiResponse,
  AuthLoginResponse,
  CoreInfo,
  CoreStatus,
  LogResponse,
  MergedConfig,
  Settings,
  Subscription,
  SubscriptionListResponse,
  UserProfileDetail,
  UserProfileListResponse,
  UserProfileSummary,
} from './types'
import AppHeader from './components/AppHeader'
import LoginPanel from './components/LoginPanel'
import SystemStatusSection from './components/SystemStatusSection'
import SubscriptionsSection from './components/SubscriptionsSection'
import CoreSection from './components/CoreSection'
import UserProfilesSection from './components/UserProfilesSection'
import LogsSection from './components/LogsSection'

function App() {
  const [subscriptions, setSubscriptions] = useState<Subscription[]>([])
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [name, setName] = useState('')
  const [url, setUrl] = useState('')
  const [coreInfo, setCoreInfo] = useState<CoreInfo | null>(null)
  const [coreStatus, setCoreStatus] = useState<CoreStatus | null>(null)
  const [coreLoading, setCoreLoading] = useState(false)
  const [coreActionLoading, setCoreActionLoading] = useState(false)
  const [authToken, setAuthToken] = useState<string | null>(null)
  const [authReady, setAuthReady] = useState(false)
  const [passwordSet, setPasswordSet] = useState(false)
  const [loginPassword, setLoginPassword] = useState('')
  const [loginLoading, setLoginLoading] = useState(false)
  const [userProfiles, setUserProfiles] = useState<UserProfileSummary[]>([])
  const [userProfilesLoading, setUserProfilesLoading] = useState(false)
  const [activeUserProfileId, setActiveUserProfileId] = useState<string | null>(null)
  const [editingUserProfileId, setEditingUserProfileId] = useState<string | null>(null)
  const [userProfileName, setUserProfileName] = useState('')
  const [userProfileContent, setUserProfileContent] = useState('')
  const [userProfileSaving, setUserProfileSaving] = useState(false)
  const [newUserProfileName, setNewUserProfileName] = useState('')
  const [creatingUserProfile, setCreatingUserProfile] = useState(false)
  const [mergedConfig, setMergedConfig] = useState<string>('')
  const [mergedConfigLoading, setMergedConfigLoading] = useState(false)
  const [appLog, setAppLog] = useState<string[]>([])
  const [mihomoLog, setMihomoLog] = useState<string[]>([])
  const [logLoading, setLogLoading] = useState(false)

  const authedFetch = (input: RequestInfo | URL, init?: RequestInit) => {
    const headers = new Headers(init?.headers ?? {})
    if (authToken) {
      headers.set('X-Auth-Token', authToken)
    }
    return fetch(input, { ...init, headers })
  }

  const resetForm = () => {
    setEditingId(null)
    setName('')
    setUrl('')
  }

  const loadSubscriptions = async () => {
    setLoading(true)
    setError(null)
    try {
      const res = await authedFetch('/api/subscriptions')
      const body = (await res.json()) as ApiResponse<SubscriptionListResponse>
      if (body.code !== 'ok' || !body.data) {
        throw new Error(body.message || '加载订阅列表失败')
      }
      setSubscriptions(body.data.subscriptions)
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载订阅列表失败')
    } finally {
      setLoading(false)
    }
  }

  const refreshCore = async () => {
    setCoreLoading(true)
    try {
      const [infoRes, statusRes] = await Promise.all([
        authedFetch('/api/core'),
        authedFetch('/api/core/status'),
      ])
      const infoBody = (await infoRes.json()) as ApiResponse<CoreInfo>
      const statusBody = (await statusRes.json()) as ApiResponse<CoreStatus>
      if (infoBody.code === 'ok' && infoBody.data) {
        setCoreInfo(infoBody.data)
      }
      if (statusBody.code === 'ok' && statusBody.data) {
        setCoreStatus(statusBody.data)
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载内核信息失败')
    } finally {
      setCoreLoading(false)
    }
  }

  const loadUserProfiles = async () => {
    setUserProfilesLoading(true)
    try {
      const res = await authedFetch('/api/user-profiles')
      const body = (await res.json()) as ApiResponse<UserProfileListResponse>
      if (body.code !== 'ok' || !body.data) {
        throw new Error(body.message || '加载用户配置列表失败')
      }
      setUserProfiles(body.data.user_profiles)
      const active = body.data.user_profiles.find((p) => p.is_active)
      setActiveUserProfileId(active ? active.id : null)
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载用户配置列表失败')
    } finally {
      setUserProfilesLoading(false)
    }
  }

  const loadUserProfileDetail = async (id: string) => {
    setError(null)
    setMessage(null)
    try {
      const res = await authedFetch(`/api/user-profiles/${encodeURIComponent(id)}`)
      const body = (await res.json()) as ApiResponse<UserProfileDetail>
      if (body.code !== 'ok' || !body.data) {
        throw new Error(body.message || '加载用户配置失败')
      }
      const detail = body.data
      setEditingUserProfileId(detail.id)
      setUserProfileName(detail.name)
      setUserProfileContent(detail.content)
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载用户配置失败')
    }
  }

  const loadMergedConfig = async () => {
    setMergedConfigLoading(true)
    try {
      const res = await authedFetch('/api/config/merged')
      const body = (await res.json()) as ApiResponse<MergedConfig>
      if (body.code === 'ok' && body.data) {
        setMergedConfig(body.data.content)
      } else if (body.code === 'merged_config_not_found') {
        setMergedConfig('# 当前还没有生成 merged.yaml\n')
      } else if (body.message) {
        setError(body.message)
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载合并配置失败')
    } finally {
      setMergedConfigLoading(false)
    }
  }

  const loadSettings = async (tokenFromStorage: string | null) => {
    try {
      if (tokenFromStorage) {
        setAuthToken(tokenFromStorage)
      }
      const res = await authedFetch('/api/settings')
      const body = (await res.json()) as ApiResponse<Settings>
      if (body.code === 'unauthorized') {
        // token 无效或过期
        setAuthToken(null)
        localStorage.removeItem('freeNetworkToken')
        setPasswordSet(true)
      } else if (body.code === 'ok' && body.data) {
        setPasswordSet(body.data.password_set)
      } else if (body.message) {
        setError(body.message)
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载设置失败')
    } finally {
      setAuthReady(true)
    }
  }

  const loadLogs = async () => {
    setLogLoading(true)
    try {
      const [appRes, mihomoRes] = await Promise.all([
        authedFetch('/api/logs/app'),
        authedFetch('/api/logs/mihomo'),
      ])
      const appBody = (await appRes.json()) as ApiResponse<LogResponse>
      const mihomoBody = (await mihomoRes.json()) as ApiResponse<LogResponse>

      if (appBody.code === 'ok' && appBody.data) {
        setAppLog(appBody.data.lines)
      } else if (appBody.code === 'log_not_found') {
        setAppLog([])
      } else if (appBody.message) {
        setError(appBody.message)
      }

      if (mihomoBody.code === 'ok' && mihomoBody.data) {
        setMihomoLog(mihomoBody.data.lines)
      } else if (mihomoBody.code === 'log_not_found') {
        setMihomoLog([])
      } else if (mihomoBody.message) {
        setError(mihomoBody.message)
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载日志失败')
    } finally {
      setLogLoading(false)
    }
  }

  useEffect(() => {
    const storedToken = localStorage.getItem('freeNetworkToken')
    void loadSettings(storedToken)
  }, [])

  useEffect(() => {
    if (!authReady) return
    if (passwordSet && !authToken) return
    void loadSubscriptions()
    void refreshCore()
    void loadUserProfiles()
    void loadMergedConfig()
    void loadLogs()
  }, [authReady, passwordSet, authToken])

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault()
    if (!name.trim() || !url.trim()) {
      setError('名称和 URL 均不能为空')
      return
    }
    setSaving(true)
    setError(null)
    setMessage(null)
    try {
      const payload = { name: name.trim(), url: url.trim() }
      let res: Response
      if (editingId) {
        res = await authedFetch(
          `/api/subscriptions/${encodeURIComponent(editingId)}`,
          {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
          },
        )
      } else {
        res = await authedFetch('/api/subscriptions', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload),
        })
      }
      const body = (await res.json()) as ApiResponse<Subscription>
      if (body.code !== 'ok') {
        throw new Error(body.message || '保存订阅失败')
      }
      setMessage(editingId ? '订阅已更新' : '订阅已创建')
      resetForm()
      await loadSubscriptions()
    } catch (err) {
      setError(err instanceof Error ? err.message : '保存订阅失败')
    } finally {
      setSaving(false)
    }
  }

  const handleEdit = (sub: Subscription) => {
    setEditingId(sub.id)
    setName(sub.name)
    setUrl(sub.url)
    setMessage(null)
    setError(null)
  }

  const handleDelete = async (id: string) => {
    if (!window.confirm('确认删除该订阅？')) return
    setError(null)
    setMessage(null)
    try {
      const res = await authedFetch(`/api/subscriptions/${encodeURIComponent(id)}`, {
        method: 'DELETE',
      })
      const body = (await res.json()) as ApiResponse<unknown>
      if (body.code !== 'ok') {
        throw new Error(body.message || '删除订阅失败')
      }
      setMessage('订阅已删除')
      await loadSubscriptions()
    } catch (err) {
      setError(err instanceof Error ? err.message : '删除订阅失败')
    }
  }

  const handleActivate = async (id: string) => {
    setError(null)
    setMessage(null)
    try {
      const res = await authedFetch(
        `/api/subscriptions/${encodeURIComponent(id)}/activate`,
        {
          method: 'POST',
        },
      )
      const body = (await res.json()) as ApiResponse<unknown>
      if (body.code !== 'ok') {
        throw new Error(body.message || '设置活跃订阅失败')
      }
      setMessage('已设置当前订阅')
      await loadSubscriptions()
    } catch (err) {
      setError(err instanceof Error ? err.message : '设置活跃订阅失败')
    }
  }

  const handleFetch = async (id: string) => {
    setError(null)
    setMessage(null)
    try {
      const res = await authedFetch(
        `/api/subscriptions/${encodeURIComponent(id)}/fetch`,
        {
          method: 'POST',
        },
      )
      const body = (await res.json()) as ApiResponse<unknown>
      if (body.code !== 'ok') {
        throw new Error(body.message || '拉取订阅失败')
      }
      setMessage('订阅内容已拉取并保存')
      await loadSubscriptions()
    } catch (err) {
      setError(err instanceof Error ? err.message : '拉取订阅失败')
    }
  }

  const handleCoreDownload = async () => {
    setCoreActionLoading(true)
    setError(null)
    setMessage(null)
    try {
      const res = await authedFetch('/api/core/download', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      })
      const body = (await res.json()) as ApiResponse<CoreInfo>
      if (body.code !== 'ok') {
        throw new Error(body.message || '内核下载失败')
      }
      if (body.data) {
        setCoreInfo(body.data)
      }
      setMessage('内核已下载并安装')
    } catch (err) {
      setError(err instanceof Error ? err.message : '内核下载失败')
    } finally {
      setCoreActionLoading(false)
    }
  }

  const handleCoreStart = async () => {
    setCoreActionLoading(true)
    setError(null)
    setMessage(null)
    try {
      const res = await authedFetch('/api/core/start', { method: 'POST' })
      const body = (await res.json()) as ApiResponse<{ pid?: number }>
      if (body.code !== 'ok') {
        throw new Error(body.message || '启动内核失败')
      }
      setMessage('内核已启动')
      await refreshCore()
    } catch (err) {
      setError(err instanceof Error ? err.message : '启动内核失败')
    } finally {
      setCoreActionLoading(false)
    }
  }

  const handleCoreStop = async () => {
    setCoreActionLoading(true)
    setError(null)
    setMessage(null)
    try {
      const res = await authedFetch('/api/core/stop', { method: 'POST' })
      const body = (await res.json()) as ApiResponse<unknown>
      if (body.code !== 'ok') {
        throw new Error(body.message || '停止内核失败')
      }
      setMessage('内核已停止')
      await refreshCore()
    } catch (err) {
      setError(err instanceof Error ? err.message : '停止内核失败')
    } finally {
      setCoreActionLoading(false)
    }
  }

  const handleCreateUserProfile = async () => {
    if (!newUserProfileName.trim()) {
      setError('用户 profile 名称不能为空')
      return
    }
    setCreatingUserProfile(true)
    setError(null)
    setMessage(null)
    try {
      const res = await authedFetch('/api/user-profiles', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: newUserProfileName.trim(),
          content: userProfileContent,
        }),
      })
      const body = (await res.json()) as ApiResponse<UserProfileSummary>
      if (body.code !== 'ok' || !body.data) {
        throw new Error(body.message || '创建用户配置失败')
      }
      setMessage('用户配置已创建')
      setNewUserProfileName('')
      await loadUserProfiles()
      await loadUserProfileDetail(body.data.id)
      await loadMergedConfig()
    } catch (err) {
      setError(err instanceof Error ? err.message : '创建用户配置失败')
    } finally {
      setCreatingUserProfile(false)
    }
  }

  const handleSaveUserProfile = async () => {
    if (!editingUserProfileId) {
      setError('请先在左侧选择一个用户 profile')
      return
    }
    if (!userProfileName.trim()) {
      setError('用户 profile 名称不能为空')
      return
    }
    setUserProfileSaving(true)
    setError(null)
    setMessage(null)
    try {
      const res = await authedFetch(
        `/api/user-profiles/${encodeURIComponent(editingUserProfileId)}`,
        {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            name: userProfileName.trim(),
            content: userProfileContent,
          }),
        },
      )
      const body = (await res.json()) as ApiResponse<UserProfileDetail>
      if (body.code !== 'ok' || !body.data) {
        throw new Error(body.message || '保存用户配置失败')
      }
      setMessage('用户配置已保存并合并')
      await loadUserProfiles()
      await loadMergedConfig()
    } catch (err) {
      setError(err instanceof Error ? err.message : '保存用户配置失败')
    } finally {
      setUserProfileSaving(false)
    }
  }

  const handleActivateUserProfile = async (id: string) => {
    setError(null)
    setMessage(null)
    try {
      const res = await authedFetch(
        `/api/user-profiles/${encodeURIComponent(id)}/activate`,
        {
          method: 'POST',
        },
      )
      const body = (await res.json()) as ApiResponse<unknown>
      if (body.code !== 'ok') {
        throw new Error(body.message || '设置活跃用户配置失败')
      }
      setMessage('已设置当前用户配置')
      setActiveUserProfileId(id)
      await loadUserProfiles()
      await loadMergedConfig()
    } catch (err) {
      setError(err instanceof Error ? err.message : '设置活跃用户配置失败')
    }
  }

  const handleDeleteUserProfile = async (id: string) => {
    if (!window.confirm('确认删除该用户 profile？')) return
    setError(null)
    setMessage(null)
    try {
      const res = await authedFetch(`/api/user-profiles/${encodeURIComponent(id)}`, {
        method: 'DELETE',
      })
      const body = (await res.json()) as ApiResponse<unknown>
      if (body.code !== 'ok') {
        throw new Error(body.message || '删除用户配置失败')
      }
      setMessage('用户配置已删除')
      if (editingUserProfileId === id) {
        setEditingUserProfileId(null)
        setUserProfileName('')
        setUserProfileContent('')
      }
      await loadUserProfiles()
      await loadMergedConfig()
    } catch (err) {
      setError(err instanceof Error ? err.message : '删除用户配置失败')
    }
  }

  const handleLogin = async (e: FormEvent) => {
    e.preventDefault()
    if (!loginPassword.trim()) {
      setError('请输入面板访问密码')
      return
    }
    setLoginLoading(true)
    setError(null)
    setMessage(null)
    try {
      const res = await fetch('/api/auth/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ password: loginPassword }),
      })
      const body = (await res.json()) as ApiResponse<AuthLoginResponse>
      if (body.code !== 'ok' || !body.data) {
        throw new Error(body.message || '登录失败')
      }
      setAuthToken(body.data.token)
      localStorage.setItem('freeNetworkToken', body.data.token)
      setLoginPassword('')
      setMessage('登录成功')
      void loadSubscriptions()
      void refreshCore()
      void loadUserProfiles()
      void loadMergedConfig()
      void loadLogs()
    } catch (err) {
      setError(err instanceof Error ? err.message : '登录失败')
    } finally {
      setLoginLoading(false)
    }
  }

  const handleLogout = () => {
    setAuthToken(null)
    localStorage.removeItem('freeNetworkToken')
    setMessage('已退出登录')
  }

  return (
    <div className="app-root bg-slate-950 text-slate-100">
      <div className="mx-auto flex min-h-screen max-w-5xl flex-col px-4 py-8">
        <AppHeader
          passwordSet={passwordSet}
          authToken={authToken}
          onLogout={handleLogout}
        />

        {authReady && passwordSet && !authToken ? (
          <LoginPanel
            loginPassword={loginPassword}
            onLoginPasswordChange={setLoginPassword}
            onSubmit={handleLogin}
            loading={loginLoading}
            error={error}
            message={message}
          />
        ) : (
          <main className="flex flex-1 flex-col gap-4">
            <SystemStatusSection
              coreStatus={coreStatus}
              subscriptionsCount={subscriptions.length}
              passwordSet={passwordSet}
            />

          <SubscriptionsSection
            subscriptions={subscriptions}
            loading={loading}
            saving={saving}
            editingId={editingId}
            name={name}
            url={url}
            message={message}
            error={error}
            onChangeName={setName}
            onChangeUrl={setUrl}
            onResetForm={resetForm}
            onSubmit={handleSubmit}
            onReload={() => {
              void loadSubscriptions()
            }}
            onEdit={handleEdit}
            onDelete={handleDelete}
            onActivate={handleActivate}
            onFetch={(id) => {
              void handleFetch(id)
            }}
          />

          <section className="grid gap-4 md:grid-cols-2">
            <CoreSection
              coreInfo={coreInfo}
              coreStatus={coreStatus}
              coreLoading={coreLoading}
              coreActionLoading={coreActionLoading}
              onRefresh={() => {
                void refreshCore()
              }}
              onDownload={() => {
                void handleCoreDownload()
              }}
              onStart={() => {
                void handleCoreStart()
              }}
              onStop={() => {
                void handleCoreStop()
              }}
            />
            <UserProfilesSection
              userProfiles={userProfiles}
              userProfilesLoading={userProfilesLoading}
              activeUserProfileId={activeUserProfileId}
              userProfileName={userProfileName}
              userProfileContent={userProfileContent}
              userProfileSaving={userProfileSaving}
              newUserProfileName={newUserProfileName}
              creatingUserProfile={creatingUserProfile}
              mergedConfig={mergedConfig}
              mergedConfigLoading={mergedConfigLoading}
              onReloadUserProfiles={() => {
                void loadUserProfiles()
              }}
              onLoadUserProfileDetail={(id) => {
                void loadUserProfileDetail(id)
              }}
              onActivateUserProfile={(id) => {
                void handleActivateUserProfile(id)
              }}
              onDeleteUserProfile={handleDeleteUserProfile}
              onNewUserProfileNameChange={setNewUserProfileName}
              onCreateUserProfile={() => {
                void handleCreateUserProfile()
              }}
              onUserProfileNameChange={setUserProfileName}
              onUserProfileContentChange={setUserProfileContent}
              onSaveUserProfile={() => {
                void handleSaveUserProfile()
              }}
              onReloadMergedConfig={() => {
                void loadMergedConfig()
              }}
            />
          </section>

          <LogsSection
            appLog={appLog}
            mihomoLog={mihomoLog}
            loading={logLoading}
            onReload={() => {
              void loadLogs()
            }}
          />
        </main>
        )}

        <footer className="mt-6 border-t border-slate-800 pt-4">
          <p className="text-xs text-slate-500">
            当前进度：已完成里程碑 1–5（HTTP 服务与静态页面、订阅管理、内核下载与运行控制、用户 profile 管理与配置合并、面板访问密码与日志监控）。
          </p>
        </footer>
      </div>
    </div>
  )
}

export default App
