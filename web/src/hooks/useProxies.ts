import { useCallback, useState } from 'react'
import type { ProxiesView } from '../types'
import { useAuth } from '../context/AuthContext'
import { useNotifications } from '../context/NotificationContext'
import {
  getProxies,
  selectProxyNode,
  testProxyNode,
} from '../api'

export function useProxies() {
  const { authedFetch } = useAuth()
  const { notifyError, notifySuccess } = useNotifications()

  const [view, setView] = useState<ProxiesView | null>(null)
  const [loading, setLoading] = useState(false)
  const [selecting, setSelecting] = useState(false)
  const [testing, setTesting] = useState(false)
  const [testingNodes, setTestingNodes] = useState<Record<string, boolean>>({})

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

      const current = view
      if (!current) {
        notifyError('当前无代理组数据，无法测试延迟')
        return
      }
      const group = current.groups.find((g) => g.name === groupName)
      if (!group) {
        notifyError(`未找到代理组 ${groupName}`)
        return
      }

      const nodes = group.nodes
      if (!nodes.length) {
        notifyError(`代理组 ${groupName} 下暂无节点`)
        return
      }

      setTesting(true)
      let firstError: string | null = null

      try {
        const concurrency = 5
        let index = 0

        const runWorker = async () => {
          // eslint-disable-next-line no-constant-condition
          while (true) {
            const currentIndex = index
            if (currentIndex >= nodes.length) break
            index += 1

            const node = nodes[currentIndex]
            const key = `${groupName}::${node.name}`

            setTestingNodes((prev) => ({ ...prev, [key]: true }))
            try {
              const resp = await testProxyNode(
                authedFetch,
                groupName,
                node.name,
              )
              const delayMs = resp.delay_ms
              setView((prev) => {
                if (!prev) return prev
                const groups = prev.groups.map((g) => {
                  if (g.name !== groupName) return g
                  return {
                    ...g,
                    nodes: g.nodes.map((n) =>
                      n.name === node.name ? { ...n, delay: delayMs } : n,
                    ),
                  }
                })
                return { ...prev, groups }
              })
            } catch (err) {
              console.error(
                '测试节点延迟失败',
                groupName,
                node.name,
                err,
              )
              if (!firstError) {
                firstError =
                  err instanceof Error
                    ? err.message
                    : '测试节点延迟失败'
              }
            } finally {
              setTestingNodes((prev) => {
                const next = { ...prev }
                delete next[key]
                return next
              })
            }
          }
        }

        const workerCount = Math.min(concurrency, nodes.length)
        await Promise.all(
          Array.from({ length: workerCount }, () => runWorker()),
        )

        if (firstError) {
          notifyError(firstError)
        } else {
          notifySuccess(`已完成代理组 ${groupName} 的延迟测试`)
        }
      } finally {
        setTesting(false)
      }
    },
    [authedFetch, notifyError, notifySuccess, view],
  )

  return {
    view,
    loading,
    selecting,
    testing,
    testingNodes,
    load,
    select,
    testGroup,
  }
}
