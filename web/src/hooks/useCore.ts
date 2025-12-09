import { useCallback, useState } from 'react'
import type { CoreInfo, CoreStatus } from '../types'
import { useAuth } from '../context/AuthContext'
import { useNotifications } from '../context/NotificationContext'
import {
  downloadCore,
  getCoreInfo,
  getCoreStatus,
  startCore,
  stopCore,
} from '../api'

export function useCore() {
  const { authedFetch } = useAuth()
  const { notifyError, notifySuccess } = useNotifications()

  const [coreInfo, setCoreInfo] = useState<CoreInfo | null>(null)
  const [coreStatus, setCoreStatus] = useState<CoreStatus | null>(null)
  const [loading, setLoading] = useState(false)
  const [actionLoading, setActionLoading] = useState(false)

  const refresh = useCallback(async () => {
    setLoading(true)
    try {
      const [info, status] = await Promise.all([
        getCoreInfo(authedFetch),
        getCoreStatus(authedFetch),
      ])
      if (info) setCoreInfo(info)
      if (status) setCoreStatus(status)
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : '加载内核信息失败'
      notifyError(msg)
    } finally {
      setLoading(false)
    }
  }, [authedFetch, notifyError])

  const download = useCallback(async () => {
    setActionLoading(true)
    try {
      const info = await downloadCore(authedFetch)
      setCoreInfo(info)
      notifySuccess('内核已下载并安装')
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : '内核下载失败'
      notifyError(msg)
    } finally {
      setActionLoading(false)
    }
  }, [authedFetch, notifyError, notifySuccess])

  const start = useCallback(async () => {
    setActionLoading(true)
    try {
      await startCore(authedFetch)
      notifySuccess('内核已启动')
      await refresh()
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : '启动内核失败'
      notifyError(msg)
    } finally {
      setActionLoading(false)
    }
  }, [authedFetch, notifyError, notifySuccess, refresh])

  const stop = useCallback(async () => {
    setActionLoading(true)
    try {
      await stopCore(authedFetch)
      notifySuccess('内核已停止')
      await refresh()
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : '停止内核失败'
      notifyError(msg)
    } finally {
      setActionLoading(false)
    }
  }, [authedFetch, notifyError, notifySuccess, refresh])

  return {
    coreInfo,
    coreStatus,
    loading,
    actionLoading,
    refresh,
    download,
    start,
    stop,
  }
}

