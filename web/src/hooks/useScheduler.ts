import { useCallback, useState } from 'react'
import type { ScheduledTaskConfig } from '../types'
import { useAuth } from '../context/AuthContext'
import { useNotifications } from '../context/NotificationContext'
import { fetchSettings, updateSchedulerSettings } from '../api'

export function useScheduler() {
  const { authedFetch } = useAuth()
  const { notifyError, notifySuccess } = useNotifications()

  const [subscriptionTask, setSubscriptionTask] =
    useState<ScheduledTaskConfig | null>(null)
  const [geoipTask, setGeoipTask] = useState<ScheduledTaskConfig | null>(null)
  const [saving, setSaving] = useState(false)

  const load = useCallback(async () => {
    try {
      const body = await fetchSettings(authedFetch)
      if (body.code === 'ok' && body.data) {
        setSubscriptionTask(body.data.subscription_auto_update ?? null)
        setGeoipTask(body.data.geoip_auto_update ?? null)
      } else if (body.message) {
        notifyError(body.message)
      }
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : '加载设置失败'
      notifyError(msg)
    }
  }, [authedFetch, notifyError])

  const save = useCallback(async () => {
    setSaving(true)
    try {
      const data = await updateSchedulerSettings(authedFetch, {
        subscriptionTask,
        geoipTask,
      })
      setSubscriptionTask(data.subscription_auto_update ?? null)
      setGeoipTask(data.geoip_auto_update ?? null)
      notifySuccess('自动更新计划已保存')
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : '保存计划任务设置失败'
      notifyError(msg)
    } finally {
      setSaving(false)
    }
  }, [authedFetch, geoipTask, notifyError, notifySuccess, subscriptionTask])

  return {
    subscriptionTask,
    geoipTask,
    saving,
    setSubscriptionTask,
    setGeoipTask,
    load,
    save,
  }
}

