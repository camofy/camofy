import { useCallback, useState } from 'react'
import type { Subscription } from '../types'
import { useAuth } from '../context/AuthContext'
import { useNotifications } from '../context/NotificationContext'
import {
  activateSubscription,
  deleteSubscription,
  fetchSubscriptionContent,
  listSubscriptions,
  saveSubscription,
} from '../api'

export function useSubscriptions() {
  const { authedFetch } = useAuth()
  const { notifyError, notifySuccess } = useNotifications()
  const [subscriptions, setSubscriptions] = useState<Subscription[]>([])
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const list = await listSubscriptions(authedFetch)
      setSubscriptions(list)
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : '加载订阅列表失败'
      notifyError(msg)
    } finally {
      setLoading(false)
    }
  }, [authedFetch, notifyError])

  const save = useCallback(
    async (params: { id?: string | null; name: string; url: string }) => {
      if (!params.name.trim() || !params.url.trim()) {
        notifyError('名称和 URL 均不能为空')
        return
      }
      setSaving(true)
      try {
        const created = await saveSubscription(authedFetch, params)
        notifySuccess(params.id ? '订阅已更新' : '订阅已创建')
        setSubscriptions((prev) => {
          const others = prev.filter((s) => s.id !== created.id)
          return [...others, created]
        })
        await load()
      } catch (err) {
        const msg =
          err instanceof Error ? err.message : '保存订阅失败'
        notifyError(msg)
      } finally {
        setSaving(false)
      }
    },
    [authedFetch, load, notifyError, notifySuccess],
  )

  const remove = useCallback(
    async (id: string) => {
      try {
        await deleteSubscription(authedFetch, id)
        notifySuccess('订阅已删除')
        await load()
      } catch (err) {
        const msg =
          err instanceof Error ? err.message : '删除订阅失败'
        notifyError(msg)
      }
    },
    [authedFetch, load, notifyError, notifySuccess],
  )

  const activate = useCallback(
    async (id: string) => {
      try {
        await activateSubscription(authedFetch, id)
        notifySuccess('已设置当前订阅')
        await load()
      } catch (err) {
        const msg =
          err instanceof Error ? err.message : '设置活跃订阅失败'
        notifyError(msg)
      }
    },
    [authedFetch, load, notifyError, notifySuccess],
  )

  const fetchRemote = useCallback(
    async (id: string) => {
      try {
        await fetchSubscriptionContent(authedFetch, id)
        notifySuccess('订阅内容已拉取并保存')
        await load()
      } catch (err) {
        const msg =
          err instanceof Error ? err.message : '拉取订阅失败'
        notifyError(msg)
      }
    },
    [authedFetch, load, notifyError, notifySuccess],
  )

  return {
    subscriptions,
    loading,
    saving,
    load,
    save,
    remove,
    activate,
    fetchRemote,
  }
}

