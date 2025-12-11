import type { CoreInfo, CoreOperationState, CoreStatus } from '../types'

type CoreSectionProps = {
  coreInfo: CoreInfo | null
  coreStatus: CoreStatus | null
  coreLoading: boolean
  coreActionLoading: boolean
  coreOperation: CoreOperationState | null
  onRefresh: () => void
  onDownload: () => void
  onStart: () => void
  onStop: () => void
}

function CoreSection({
  coreInfo,
  coreStatus,
  coreLoading,
  coreActionLoading,
  coreOperation,
  onRefresh,
  onDownload,
  onStart,
  onStop,
}: CoreSectionProps) {
  const downloadRunning =
    coreOperation?.kind === 'download' &&
    coreOperation.status === 'running'

  return (
    <div className="rounded-lg border border-slate-800 bg-slate-900/60 p-4">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold text-slate-100">内核管理</h3>
        <button
          type="button"
          className="rounded border border-slate-700 bg-slate-900 px-2 py-1 text-[11px] text-slate-200 hover:bg-slate-800"
          onClick={onRefresh}
        >
          刷新
        </button>
      </div>
      {coreLoading ? (
        <p className="mt-2 text-xs text-slate-400">正在加载内核信息…</p>
      ) : (
        <div className="mt-2 space-y-2 text-xs text-slate-300">
          <p>
            推荐架构：{' '}
            <span className="font-mono">
              {coreInfo?.recommended_arch || '未知'}
            </span>
          </p>
          <p>
            当前内核：{' '}
            {coreInfo?.binary_exists ? (
              <span className="text-emerald-400">
                已安装
                {coreInfo?.version ? `（版本：${coreInfo.version}）` : ''}
              </span>
            ) : (
              <span className="text-slate-400">未安装</span>
            )}
          </p>
          {coreInfo?.last_download_time && (
            <p className="text-slate-400">
              最近下载时间：{coreInfo.last_download_time}
            </p>
          )}
          <p>
            运行状态：{' '}
            {coreStatus?.running ? (
              <span className="text-emerald-400">
                运行中
                {coreStatus.pid ? `（PID：${coreStatus.pid}）` : ''}
              </span>
            ) : (
              <span className="text-slate-400">未运行</span>
            )}
          </p>
        </div>
      )}

      <div className="mt-3 space-y-2">
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            disabled={coreActionLoading || downloadRunning}
            onClick={onDownload}
            className="rounded bg-sky-600 px-3 py-1.5 text-[11px] font-medium text-white hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
          >
            下载 / 更新内核
          </button>
          <button
            type="button"
            disabled={coreActionLoading || coreStatus?.running}
            onClick={onStart}
            className="rounded border border-emerald-700 bg-slate-900 px-3 py-1.5 text-[11px] text-emerald-300 hover:bg-emerald-800/40 disabled:cursor-not-allowed disabled:opacity-60"
          >
            启动内核
          </button>
          <button
            type="button"
            disabled={coreActionLoading || !coreStatus?.running}
            onClick={onStop}
            className="rounded border border-red-700 bg-slate-900 px-3 py-1.5 text-[11px] text-red-300 hover:bg-red-800/40 disabled:cursor-not-allowed disabled:opacity-60"
          >
            停止内核
          </button>
        </div>
        {coreOperation &&
          coreOperation.kind === 'download' &&
          coreOperation.status === 'running' && (
            <div className="mt-2 space-y-1">
              <p className="text-[11px] text-slate-400">
                正在下载 / 更新内核…
                {typeof coreOperation.progress === 'number'
                  ? ` (${Math.round(coreOperation.progress * 100)}%)`
                  : null}
              </p>
              {typeof coreOperation.progress === 'number' && (
                <div className="h-1.5 w-full overflow-hidden rounded-full bg-slate-800">
                  <div
                    className="h-full bg-sky-500 transition-[width]"
                    style={{
                      width: `${Math.max(
                        0,
                        Math.min(100, coreOperation.progress * 100),
                      )}%`,
                    }}
                  />
                </div>
              )}
            </div>
          )}
      </div>
    </div>
  )
}

export default CoreSection
