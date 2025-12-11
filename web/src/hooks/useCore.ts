import { useCallback, useEffect, useRef, useState } from 'react'
import type {
  AppEvent,
  CoreInfo,
  CoreOperationState,
  CoreStatus,
} from '../types'
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
  const { authedFetch, token, authReady } = useAuth()
  const { notifyError, notifySuccess } = useNotifications()

  const [coreInfo, setCoreInfo] = useState<CoreInfo | null>(null)
  const [coreStatus, setCoreStatus] = useState<CoreStatus | null>(null)
  const [loading, setLoading] = useState(false)
  const [actionLoading, setActionLoading] = useState(false)
  const [operationState, setOperationState] = useState<CoreOperationState | null>(null)
  const wsRef = useRef<WebSocket | null>(null)

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
    try {
      await startCore(authedFetch)
      notifySuccess('已提交内核启动请求')
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : '启动内核失败'
      notifyError(msg)
    }
  }, [authedFetch, notifyError, notifySuccess, refresh])

  const stop = useCallback(async () => {
    try {
      await stopCore(authedFetch)
      notifySuccess('已提交内核停止请求')
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : '停止内核失败'
      notifyError(msg)
    }
  }, [authedFetch, notifyError, notifySuccess, refresh])

  // 通过 WebSocket 订阅后端推送的核心状态/操作事件（包括下载进度）
  useEffect(() => {
    if (!authReady) return

    const protocol = window.location.protocol === 'https:' ? 'wss' : 'ws'
    const host = window.location.host
    const tokenParam = token ? `?token=${encodeURIComponent(token)}` : ''
    const url = `${protocol}://${host}/api/events/ws${tokenParam}`

    const ws = new WebSocket(url)
    wsRef.current = ws

    ws.onmessage = (event) => {
      try {
        const payload = JSON.parse(event.data) as AppEvent
        if (payload.type === 'core_status_changed') {
          setCoreStatus({
            running: payload.running,
            pid: payload.pid ?? null,
          })
        } else if (payload.type === 'core_operation_updated') {
          setOperationState(payload.state)
          const isCoreAction =
            payload.state.kind === 'start' ||
            payload.state.kind === 'stop'
          setActionLoading(
            isCoreAction && payload.state.status === 'running',
          )
          if (payload.state.status === 'success') {
            const op =
              payload.state.kind === 'start'
                ? '启动'
                : payload.state.kind === 'stop'
                  ? '停止'
                  : '下载 / 更新'
            notifySuccess(
              payload.state.message || `内核${op}完成`,
            )
            // 操作成功时刷新一次信息，保证 coreInfo 同步
            void refresh()
          } else if (payload.state.status === 'error') {
            const op =
              payload.state.kind === 'start'
                ? '启动'
                : payload.state.kind === 'stop'
                  ? '停止'
                  : '下载 / 更新'
            notifyError(
              payload.state.message || `内核${op}失败`,
            )
            void refresh()
          }
        }
      } catch (err) {
        console.error('failed to handle ws event', err)
      }
    }

    ws.onerror = (err) => {
      console.error('websocket error', err)
    }

    ws.onclose = () => {
      wsRef.current = null
    }

    return () => {
      ws.close()
    }
  }, [authReady, token, notifyError, notifySuccess, refresh])

  // 根据当前 operation 状态自动设置 actionLoading
  useEffect(() => {
    const isCoreAction =
      operationState &&
      (operationState.kind === 'start' || operationState.kind === 'stop')
    setActionLoading(
      !!(isCoreAction && operationState?.status === 'running'),
    )
  }, [operationState])

  return {
    coreInfo,
    coreStatus,
    loading,
    actionLoading,
    operationState,
    refresh,
    download,
    start,
    stop,
  }
}
