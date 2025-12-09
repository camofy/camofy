import { useCallback, useState } from 'react'
import { useAuth } from '../context/AuthContext'
import { useNotifications } from '../context/NotificationContext'
import { getLogs } from '../api'

export function useLogs() {
  const { authedFetch } = useAuth()
  const { notifyError } = useNotifications()

  const [appLog, setAppLog] = useState<string[]>([])
  const [mihomoLog, setMihomoLog] = useState<string[]>([])
  const [loading, setLoading] = useState(false)

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

  return {
    appLog,
    mihomoLog,
    loading,
    load,
  }
}

