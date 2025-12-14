import type { ProxiesView, ProxyGroup, ProxyNode } from '../types'

type ProxyGroupsSectionProps = {
  proxies: ProxiesView | null
  loading: boolean
  selecting: boolean
  onReload: () => void
  onSelectNode: (groupName: string, nodeName: string) => void
}

function ProxyGroupsSection({
  proxies,
  loading,
  selecting,
  onReload,
  onSelectNode,
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
    <div className="flex min-h-0 flex-1 flex-col rounded-lg border border-slate-800 bg-slate-900/60 p-4">
      <div className="flex items-center justify-between gap-2">
        <div>
          <h3 className="text-sm font-semibold text-slate-100">
            代理组与节点
          </h3>
          <p className="mt-1 text-xs text-slate-400">
            从 Mihomo 当前运行态获取代理组与节点，仅在内核运行且配置有效时可用。
          </p>
        </div>
        <button
          type="button"
          className="rounded border border-slate-700 bg-slate-900 px-2 py-1 text-[11px] text-slate-200 hover:bg-slate-800"
          onClick={onReload}
        >
          刷新
        </button>
      </div>

      <div className="mt-3 grid min-h-0 flex-1 gap-3 md:grid-cols-[12rem_1fr]">
        <div className="flex min-h-0 flex-col space-y-2">
          <p className="text-[11px] text-slate-500">
            共 {groups.length} 个代理组
          </p>
          <div className="min-h-[3rem] flex-1 min-h-0 overflow-auto rounded border border-slate-800 bg-slate-950/60 p-2">
            {loading ? (
              <p className="text-[11px] text-slate-400">正在加载代理组…</p>
            ) : groups.length === 0 ? (
              <p className="text-[11px] text-slate-500">
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
                            ? 'border border-sky-600 bg-sky-900/40 text-slate-50'
                            : 'border border-slate-800 bg-slate-900/60 text-slate-200 hover:border-sky-700 hover:bg-slate-900',
                        ].join(' ')}
                      >
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1">
                            <span className="truncate">{g.name}</span>
                            {isRuleLike && (
                              <span className="rounded bg-slate-800 px-1 text-[10px] text-slate-300">
                                {g.type}
                              </span>
                            )}
                          </div>
                          <p className="truncate text-[10px] text-slate-500">
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
            <span className="text-xs font-medium text-slate-300">
              {selectedGroup
                ? `代理组：${selectedGroup.name}（${selectedGroup.type})`
                : '代理组节点'}
            </span>
            {selecting && (
              <span className="text-[11px] text-sky-300">正在切换节点…</span>
            )}
          </div>
          <div className="flex-1 min-h-0 overflow-auto rounded border border-slate-800 bg-slate-950/60">
            {loading ? (
              <p className="p-2 text-[11px] text-slate-400">
                正在加载节点列表…
              </p>
            ) : !selectedGroup ? (
              <p className="p-2 text-[11px] text-slate-500">
                尚未选择代理组。
              </p>
            ) : selectedGroup.nodes.length === 0 ? (
              <p className="p-2 text-[11px] text-slate-500">
                当前代理组暂无节点。
              </p>
            ) : (
              <table className="min-w-full border-collapse text-[11px] text-slate-200">
                <thead>
                  <tr className="border-b border-slate-800 bg-slate-900/80">
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
                    return (
                      <tr
                        key={node.name}
                        className="border-b border-slate-800 last:border-none"
                      >
                        <td className="max-w-[10rem] px-2 py-1">
                          <div className="flex items-center gap-1">
                            <span className="truncate">{node.name}</span>
                            {isCurrent && (
                              <span className="rounded bg-emerald-700/40 px-1 text-[10px] text-emerald-200">
                                当前
                              </span>
                            )}
                          </div>
                        </td>
                        <td className="px-2 py-1 text-slate-400">
                          {node.type}
                        </td>
                        <td className="px-2 py-1 text-slate-400">
                          {typeof node.delay === 'number' && node.delay > 0
                            ? `${node.delay} ms`
                            : '未知'}
                        </td>
                        <td className="px-2 py-1 text-right">
                          <button
                            type="button"
                            disabled={isCurrent || selecting}
                            onClick={() =>
                              handleSelectNodeClick(selectedGroup, node)
                            }
                            className="rounded border border-slate-700 bg-slate-900 px-2 py-0.5 text-[10px] text-slate-200 hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50"
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

import { useState } from 'react'

export default ProxyGroupsSection
