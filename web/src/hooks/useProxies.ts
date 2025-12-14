import { useCallback, useState } from 'react'
import type { ProxiesView } from '../types'
import { useAuth } from '../context/AuthContext'
import { useNotifications } from '../context/NotificationContext'
import { getProxies, selectProxyNode, testProxyGroup } from '../api'

export function useProxies() {
  const { authedFetch } = useAuth()
  const { notifyError, notifySuccess } = useNotifications()

  const [view, setView] = useState<ProxiesView | null>(null)
  const [loading, setLoading] = useState(false)
  const [selecting, setSelecting] = useState(false)
  const [testing, setTesting] = useState(false)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const data = await getProxies(authedFetch)
      setView(data)
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : '加载代理组信息失败'
      notifyError(msg)
      setView(null)
    } finally {
      setLoading(false)
    }
  }, [authedFetch, notifyError])

  const select = useCallback(
    async (groupName: string, nodeName: string) => {
      if (!nodeName.trim()) {
        notifyError('代理节点名称不能为空')
        return
      }
      setSelecting(true)
      try {
        await selectProxyNode(authedFetch, groupName, nodeName)
        notifySuccess(`已切换代理组 ${groupName} 的节点为 ${nodeName}`)
        await load()
      } catch (err) {
        const msg =
          err instanceof Error ? err.message : '切换节点失败'
        notifyError(msg)
      } finally {
        setSelecting(false)
      }
    },
    [authedFetch, load, notifyError, notifySuccess],
  )

  const testGroup = useCallback(
    async (groupName: string) => {
      if (!groupName.trim()) return
      setTesting(true)
      try {
        await testProxyGroup(authedFetch, groupName)
        notifySuccess(`已完成代理组 ${groupName} 的延迟测试`)
        await load()
      } catch (err) {
        const msg =
          err instanceof Error ? err.message : '测试节点延迟失败'
        notifyError(msg)
      } finally {
        setTesting(false)
      }
    },
    [authedFetch, load, notifyError, notifySuccess],
  )

  return {
    view,
    loading,
    selecting,
    testing,
    load,
    select,
    testGroup,
  }
}
