import { useCallback, useState } from 'react'
import { useAuth } from '../context/AuthContext'
import { useNotifications } from '../context/NotificationContext'
import { getMergedConfig } from '../api'

export function useMergedConfig() {
  const { authedFetch } = useAuth()
  const { notifyError } = useNotifications()

  const [content, setContent] = useState<string>('')
  const [loading, setLoading] = useState(false)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const value = await getMergedConfig(authedFetch)
      if (value === null) {
        setContent('# 当前还没有生成 merged.yaml\n')
      } else {
        setContent(value)
      }
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : '加载合并配置失败'
      notifyError(msg)
    } finally {
      setLoading(false)
    }
  }, [authedFetch, notifyError])

  return {
    content,
    loading,
    load,
  }
}

