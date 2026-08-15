import { useState } from 'react'
import type { ProxiesView, ProxyGroup, ProxyNode } from '../types'

type ProxyGroupsSectionProps = {
  proxies: ProxiesView | null
  loading: boolean
  selecting: boolean
  testing: boolean
  testingNodes: Record<string, boolean>
  onReload: () => void
  onSelectNode: (groupName: string, nodeName: string) => void
  onTestGroup: (groupName: string) => void
}

function ProxyGroupsSection({
  proxies,
  loading,
  selecting,
  testing,
  testingNodes,
  onReload,
  onSelectNode,
  onTestGroup,
}: ProxyGroupsSectionProps) {
  const groups = proxies?.groups ?? []

  const [selectedGroupName, setSelectedGroupName] = useState<string | null>(
    () => (groups.length > 0 ? groups[0]?.name ?? null : null),
  )

  const selectedGroup: ProxyGroup | null =
    groups.find((g) => g.name === selectedGroupName) ?? groups[0] ?? null

  const handleSelectNodeClick = (group: ProxyGroup, node: ProxyNode) => {
    if (selecting) return
    if (group.now === node.name) return
    onSelectNode(group.name, node.name)
  }

  return (
    <div className="ui-card flex min-h-0 flex-1 flex-col p-8">
      <div className="flex items-center justify-between gap-4">
        <h3 className="sr-only">节点</h3>
        <span className="text-lg text-[color:var(--color-text-muted)]">
          {groups.length} 组
        </span>
        <button type="button" className="ui-btn-default" onClick={onReload}>
          刷新
        </button>
      </div>

      <div className="mt-6 grid min-h-0 flex-1 gap-5 md:grid-cols-[16rem_1fr]">
        <div className="min-h-[8rem] overflow-auto rounded-[24px] bg-[color:var(--color-surface-soft)] p-3">
          {loading ? (
            <p className="text-lg text-[color:var(--color-text-muted)] p-3">加载中…</p>
          ) : groups.length === 0 ? (
            <p className="text-lg text-[color:var(--color-text-muted)] p-3">暂无节点组</p>
          ) : (
            <ul className="space-y-2">
              {groups.map((g) => {
                const isActive = selectedGroup?.name === g.name
                return (
                  <li key={g.name}>
                    <button
                      type="button"
                      onClick={() => setSelectedGroupName(g.name)}
                      className={[
                        'text-lg flex w-full items-center rounded-2xl px-4 py-3 text-left',
                        isActive
                          ? 'bg-[color:var(--color-primary)] text-white'
                          : 'bg-white text-[color:var(--color-text-main)]',
                      ].join(' ')}
                    >
                      <span className="truncate">{g.name}</span>
                    </button>
                  </li>
                )
              })}
            </ul>
          )}
        </div>

        <div className="flex min-h-0 flex-col gap-4">
          <div className="flex items-center justify-between gap-3">
            <span className="text-lg text-[color:var(--color-text-muted)] truncate">
              {selectedGroup?.name ?? '未选择'}
            </span>
            <div className="flex items-center gap-3">
              {testing && (
                <span className="text-lg text-[color:var(--color-text-muted)]">测试中</span>
              )}
              {selecting && (
                <span className="text-lg text-[color:var(--color-text-muted)]">切换中</span>
              )}
              {selectedGroup && (
                <button
                  type="button"
                  disabled={loading || testing}
                  onClick={() => onTestGroup(selectedGroup.name)}
                  className="ui-btn-default"
                >
                  测速
                </button>
              )}
            </div>
          </div>
          <div className="min-h-[12rem] flex-1 overflow-auto rounded-[24px] bg-[color:var(--color-surface-soft)]">
            {loading ? (
              <p className="text-lg text-[color:var(--color-text-muted)] p-5">加载中…</p>
            ) : !selectedGroup ? (
              <p className="text-lg text-[color:var(--color-text-muted)] p-5">请选择分组</p>
            ) : selectedGroup.nodes.length === 0 ? (
              <p className="text-lg text-[color:var(--color-text-muted)] p-5">暂无节点</p>
            ) : (
              <table className="text-lg min-w-full">
                <thead>
                  <tr>
                    <th className="text-lg text-[color:var(--color-text-muted)] px-5 py-4 text-left font-medium">
                      名称
                    </th>
                    <th className="text-lg text-[color:var(--color-text-muted)] px-5 py-4 text-left font-medium">
                      延迟
                    </th>
                    <th className="text-lg text-[color:var(--color-text-muted)] px-5 py-4 text-right font-medium">
                      操作
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {selectedGroup.nodes.map((node) => {
                    const isCurrent = selectedGroup.now === node.name
                    const testingKey =
                      selectedGroup.name && node.name
                        ? `${selectedGroup.name}::${node.name}`
                        : ''
                    const isTestingNode = Boolean(testingKey && testingNodes[testingKey])
                    return (
                      <tr key={node.name} className="even:bg-white/50">
                        <td className="px-5 py-4">
                          <div className="flex items-center gap-2">
                            <span className="truncate font-medium">{node.name}</span>
                            {isCurrent && (
                              <span className="inline-flex items-center rounded-full px-3 py-1 text-lg bg-[color:var(--color-success-soft)] text-[color:var(--color-success)]">
                                当前
                              </span>
                            )}
                          </div>
                        </td>
                        <td className="px-5 py-4 text-[color:var(--color-text-muted)]">
                          {isTestingNode
                            ? '测试中'
                            : typeof node.delay === 'number' && node.delay > 0
                              ? `${node.delay} ms`
                              : '—'}
                        </td>
                        <td className="px-5 py-4 text-right">
                          <button
                            type="button"
                            disabled={isCurrent || selecting}
                            onClick={() => handleSelectNodeClick(selectedGroup, node)}
                            className="ui-btn-default"
                          >
                            {isCurrent ? '当前' : '切换'}
                          </button>
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

export default ProxyGroupsSection
