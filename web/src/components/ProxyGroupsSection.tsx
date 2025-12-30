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

  const handleSelectGroup = (name: string) => {
    setSelectedGroupName(name)
  }

  const handleSelectNodeClick = (group: ProxyGroup, node: ProxyNode) => {
    if (selecting) return
    if (group.now === node.name) return
    onSelectNode(group.name, node.name)
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col rounded-lg border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface)] p-4">
      <div className="flex items-center justify-between gap-2">
        <div>
          <h3 className="text-sm font-semibold text-[color:var(--color-text-main)]">
            代理组与节点
          </h3>
          <p className="mt-1 text-xs text-[color:var(--color-text-muted)]">
            从 Mihomo 当前运行态获取代理组与节点，仅在内核运行且配置有效时可用。
          </p>
        </div>
        <button
          type="button"
          className="rounded border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface-soft)] px-2 py-1 text-[11px] text-[color:var(--color-text-main)] hover:bg-[color:var(--color-accent)]"
          onClick={onReload}
        >
          刷新
        </button>
      </div>

      <div className="mt-3 grid min-h-0 flex-1 gap-3 md:grid-cols-[12rem_1fr]">
        <div className="flex min-h-0 flex-col space-y-2">
          <p className="text-[11px] text-[color:var(--color-text-soft)]">
            共 {groups.length} 个代理组
          </p>
          <div className="min-h-[3rem] flex-1 min-h-0 overflow-auto rounded border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface-soft)] p-2">
            {loading ? (
              <p className="text-[11px] text-[color:var(--color-text-muted)]">
                正在加载代理组…
              </p>
            ) : groups.length === 0 ? (
              <p className="text-[11px] text-[color:var(--color-text-soft)]">
                暂无可用代理组，请先确保内核已启动且配置正确。
              </p>
            ) : (
              <ul className="space-y-1">
                {groups.map((g) => {
                  const isActive = selectedGroup?.name === g.name
                  const isRuleLike =
                    g.type === 'Selector' ||
                    g.type === 'URLTest' ||
                    g.type === 'Fallback'
                  return (
                    <li key={g.name}>
                      <button
                        type="button"
                        onClick={() => handleSelectGroup(g.name)}
                        className={[
                          'flex w-full items-center justify-between gap-2 rounded px-2 py-1 text-left text-[11px]',
                          isActive
                            ? 'border border-[color:var(--color-border-strong)] bg-[color:var(--color-primary)] text-[color:var(--color-primary-on)]'
                            : 'border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface)] text-[color:var(--color-text-main)] hover:border-[color:var(--color-primary)] hover:bg-[color:var(--color-surface-soft)]',
                        ].join(' ')}
                      >
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1">
                            <span className="truncate">{g.name}</span>
                            {isRuleLike && (
                              <span className="rounded bg-[color:var(--color-accent)] px-1 text-[10px] text-[color:var(--color-text-main)]">
                                {g.type}
                              </span>
                            )}
                          </div>
                          <p className="truncate text-[10px] text-[color:var(--color-text-soft)]">
                            节点：{g.nodes.length}
                            {g.now ? ` · 当前：${g.now}` : ''}
                          </p>
                        </div>
                      </button>
                    </li>
                  )
                })}
              </ul>
            )}
          </div>
        </div>

        <div className="flex min-h-0 flex-col space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-xs font-medium text-[color:var(--color-text-main)]">
              {selectedGroup
                ? `代理组：${selectedGroup.name}（${selectedGroup.type})`
                : '代理组节点'}
            </span>
            <div className="flex items-center gap-2">
              {testing && (
                <span className="text-[11px] text-[color:var(--color-primary)]">
                  正在测试延迟…
                </span>
              )}
              {selecting && (
                <span className="text-[11px] text-[color:var(--color-success)]">
                  正在切换节点…
                </span>
              )}
              {selectedGroup && (
                <button
                  type="button"
                  disabled={loading || testing}
                  onClick={() => onTestGroup(selectedGroup.name)}
                  className="rounded border border-[color:var(--color-primary)] bg-[color:var(--color-surface-soft)] px-2 py-1 text-[10px] text-[color:var(--color-primary)] hover:bg-[color:var(--color-accent)] disabled:cursor-not-allowed disabled:opacity-60"
                >
                  测试当前组延迟
                </button>
              )}
            </div>
          </div>
          <div className="flex-1 min-h-0 overflow-auto rounded border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface-soft)]">
            {loading ? (
              <p className="p-2 text-[11px] text-[color:var(--color-text-muted)]">
                正在加载节点列表…
              </p>
            ) : !selectedGroup ? (
              <p className="p-2 text-[11px] text-[color:var(--color-text-soft)]">
                尚未选择代理组。
              </p>
            ) : selectedGroup.nodes.length === 0 ? (
              <p className="p-2 text-[11px] text-[color:var(--color-text-soft)]">
                当前代理组暂无节点。
              </p>
            ) : (
              <table className="min-w-full border-collapse text-[11px] text-[color:var(--color-text-main)]">
                <thead>
                  <tr className="border-b border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface)]">
                    <th className="px-2 py-1 text-left font-medium">节点</th>
                    <th className="px-2 py-1 text-left font-medium">类型</th>
                    <th className="px-2 py-1 text-left font-medium">延迟</th>
                    <th className="px-2 py-1 text-right font-medium">
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
                    const isTestingNode =
                      testingKey && testingNodes[testingKey]
                    return (
                      <tr
                        key={node.name}
                        className="border-b border-[color:var(--color-border-subtle)] last:border-none"
                      >
                        <td className="max-w-[10rem] px-2 py-1">
                          <div className="flex items-center gap-1">
                            <span className="truncate">{node.name}</span>
                            {isCurrent && (
                              <span className="rounded bg-[color:var(--color-success-soft)] px-1 text-[10px] text-[color:var(--color-success)]">
                                当前
                              </span>
                            )}
                          </div>
                        </td>
                        <td className="px-2 py-1 text-[color:var(--color-text-muted)]">
                          {node.type}
                        </td>
                        <td className="px-2 py-1 text-[color:var(--color-text-muted)]">
                          {isTestingNode ? (
                            <span className="inline-flex items-center gap-1 text-[color:var(--color-primary)]">
                              <span className="inline-block h-3 w-3 animate-spin rounded-full border border-[color:var(--color-primary)] border-t-transparent" />
                              测试中…
                            </span>
                          ) : typeof node.delay === 'number' && node.delay > 0 ? (
                            `${node.delay} ms`
                          ) : (
                            '未知'
                          )}
                        </td>
                        <td className="px-2 py-1 text-right">
                          <button
                            type="button"
                            disabled={isCurrent || selecting}
                            onClick={() =>
                              handleSelectNodeClick(selectedGroup, node)
                            }
                            className="rounded border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface)] px-2 py-0.5 text-[10px] text-[color:var(--color-text-main)] hover:bg-[color:var(--color-accent)] disabled:cursor-not-allowed disabled:opacity-50"
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
