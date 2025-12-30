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
  onRestart: () => void
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
  onRestart,
}: CoreSectionProps) {
  const downloadRunning =
    coreOperation?.kind === 'download' &&
    coreOperation.status === 'running'

  return (
    <div className="rounded-lg border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface)] p-4">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold text-[color:var(--color-text-main)]">内核管理</h3>
        <button
          type="button"
          className="rounded border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface-soft)] px-2 py-1 text-[11px] text-[color:var(--color-text-main)] hover:bg-[color:var(--color-accent)]"
          onClick={onRefresh}
        >
          刷新
        </button>
      </div>
      {coreLoading ? (
        <p className="mt-2 text-xs text-[color:var(--color-text-muted)]">正在加载内核信息…</p>
      ) : (
        <div className="mt-2 space-y-2 text-xs text-[color:var(--color-text-main)]">
          <p>
            推荐架构：{' '}
            <span className="font-mono">
              {coreInfo?.recommended_arch || '未知'}
            </span>
          </p>
          <p>
            当前内核：{' '}
            {coreInfo?.binary_exists ? (
              <span className="text-[color:var(--color-success)]">
                已安装
                {coreInfo?.version ? `（版本：${coreInfo.version}）` : ''}
              </span>
            ) : (
              <span className="text-[color:var(--color-text-muted)]">未安装</span>
            )}
          </p>
          {coreInfo?.last_download_time && (
            <p className="text-[color:var(--color-text-muted)]">
              最近下载时间：{coreInfo.last_download_time}
            </p>
          )}
          <p>
            运行状态：{' '}
            {coreStatus?.running ? (
              <span className="text-[color:var(--color-success)]">
                运行中
                {coreStatus.pid ? `（PID：${coreStatus.pid}）` : ''}
              </span>
            ) : (
              <span className="text-[color:var(--color-text-muted)]">未运行</span>
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
            className="rounded bg-[color:var(--color-primary)] px-3 py-1.5 text-[11px] font-medium text-[color:var(--color-primary-on)] hover:bg-[#6b6f63] disabled:cursor-not-allowed disabled:opacity-60"
          >
            下载 / 更新内核
          </button>
          <button
            type="button"
            disabled={coreActionLoading || coreStatus?.running}
            onClick={onStart}
            className="rounded border border-[color:var(--color-success)] bg-[color:var(--color-success-soft)] px-3 py-1.5 text-[11px] text-[color:var(--color-success)] hover:bg-[#c6d7bd] disabled:cursor-not-allowed disabled:opacity-60"
          >
            启动内核
          </button>
          <button
            type="button"
            disabled={coreActionLoading || !coreStatus?.running}
            onClick={onStop}
            className="rounded border border-[color:var(--color-danger)] bg-[color:var(--color-danger-soft)] px-3 py-1.5 text-[11px] text-[color:var(--color-danger)] hover:bg-[#f0c7bc] disabled:cursor-not-allowed disabled:opacity-60"
          >
            停止内核
          </button>
          <button
            type="button"
            disabled={coreActionLoading || !coreStatus?.running}
            onClick={onRestart}
            className="rounded border border-[color:var(--color-primary)] bg-[color:var(--color-surface-soft)] px-3 py-1.5 text-[11px] text-[color:var(--color-primary)] hover:bg-[color:var(--color-accent)] disabled:cursor-not-allowed disabled:opacity-60"
          >
            重启内核
          </button>
        </div>
        {coreOperation &&
          coreOperation.kind === 'download' &&
          coreOperation.status === 'running' && (
            <div className="mt-2 space-y-1">
              <p className="text-[11px] text-[color:var(--color-text-muted)]">
                正在下载 / 更新内核…
                {typeof coreOperation.progress === 'number'
                  ? ` (${Math.round(coreOperation.progress * 100)}%)`
                  : null}
              </p>
              {typeof coreOperation.progress === 'number' && (
                <div className="h-1.5 w-full overflow-hidden rounded-full bg-[color:var(--color-border-subtle)]">
                  <div
                    className="h-full bg-[color:var(--color-primary)] transition-[width]"
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
