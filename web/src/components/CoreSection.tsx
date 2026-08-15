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

function InfoItem({ label, value, tone }: { label: string; value: string; tone?: string }) {
  return (
    <div className="rounded-[24px] bg-[color:var(--color-surface-soft)] px-6 py-6">
      <p className="text-lg text-[color:var(--color-text-muted)]">{label}</p>
      <p className={`text-lg mt-3 font-semibold ${tone ?? 'text-[color:var(--color-text-main)]'}`}>
        {value}
      </p>
    </div>
  )
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
    <div className="ui-card p-8">
      <div className="flex items-center justify-between gap-4">
        <h3 className="sr-only">内核</h3>
        <button type="button" className="ui-btn-default" onClick={onRefresh}>
          刷新
        </button>
      </div>

      {coreLoading ? (
        <p className="text-lg text-[color:var(--color-text-muted)] mt-6">加载中…</p>
      ) : (
        <div className="mt-6 grid gap-5 sm:grid-cols-2 xl:grid-cols-4">
          <InfoItem label="架构" value={coreInfo?.recommended_arch || '未知'} />
          <InfoItem
            label="版本"
            value={
              coreInfo?.binary_exists
                ? coreInfo?.version || '已安装'
                : '未安装'
            }
            tone={
              coreInfo?.binary_exists
                ? 'text-[color:var(--color-success)]'
                : 'text-[color:var(--color-text-muted)]'
            }
          />
          <InfoItem
            label="下载"
            value={coreInfo?.last_download_time || '无'}
          />
          <InfoItem
            label="状态"
            value={coreStatus?.running ? '运行中' : '未运行'}
            tone={
              coreStatus?.running
                ? 'text-[color:var(--color-success)]'
                : 'text-[color:var(--color-text-muted)]'
            }
          />
        </div>
      )}

      <div className="mt-6 flex flex-wrap items-center gap-3">
        <button
          type="button"
          disabled={coreActionLoading || downloadRunning}
          onClick={onDownload}
          className="ui-btn-primary"
        >
          下载
        </button>
        <button
          type="button"
          disabled={coreActionLoading || coreStatus?.running}
          onClick={onStart}
          className="ui-btn-success"
        >
          启动
        </button>
        <button
          type="button"
          disabled={coreActionLoading || !coreStatus?.running}
          onClick={onStop}
          className="ui-btn-danger"
        >
          停止
        </button>
        <button
          type="button"
          disabled={coreActionLoading || !coreStatus?.running}
          onClick={onRestart}
          className="ui-btn-default"
        >
          重启
        </button>
      </div>

      {coreOperation &&
        coreOperation.kind === 'download' &&
        coreOperation.status === 'running' && (
          <div className="mt-6 space-y-2">
            <p className="text-lg text-[color:var(--color-text-muted)]">
              下载中
              {typeof coreOperation.progress === 'number'
                ? ` ${Math.round(coreOperation.progress * 100)}%`
                : null}
            </p>
            {typeof coreOperation.progress === 'number' && (
              <div className="h-2 w-full overflow-hidden rounded-full bg-[color:var(--color-surface-soft)]">
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
  )
}

export default CoreSection
