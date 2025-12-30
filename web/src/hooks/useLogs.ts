import { useCallback, useEffect, useRef, useState } from 'react'
import { useAuth } from '../context/AuthContext'
import { useNotifications } from '../context/NotificationContext'
import { getLogs } from '../api'
import type { AppEvent } from '../types'

export function useLogs() {
  const { authedFetch, token, authReady } = useAuth()
  const { notifyError } = useNotifications()

  const [appLog, setAppLog] = useState<string[]>([])
  const [mihomoLog, setMihomoLog] = useState<string[]>([])
  const [loading, setLoading] = useState(false)
  const wsRef = useRef<WebSocket | null>(null)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const { app, mihomo } = await getLogs(authedFetch)
      setAppLog(app)
      setMihomoLog(mihomo)
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : '加载日志失败'
      notifyError(msg)
    } finally {
      setLoading(false)
    }
  }, [authedFetch, notifyError])

  // 通过 WebSocket 实时订阅 Mihomo 日志追加内容
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
        if (payload.type === 'mihomo_log_chunk') {
          setMihomoLog((prev) => {
            const next = [...prev, payload.chunk]
            const maxLines = 1000
            if (next.length > maxLines) {
              return next.slice(next.length - maxLines)
            }
            return next
          })
        }
      } catch (err) {
        console.error('failed to handle ws event for logs', err)
      }
    }

    ws.onerror = (err) => {
      console.error('websocket error for logs', err)
    }

    ws.onclose = () => {
      wsRef.current = null
    }

    return () => {
      ws.close()
    }
  }, [authReady, token])

  return {
    appLog,
    mihomoLog,
    loading,
    load,
  }
}
