import AnsiText from './AnsiText'

type LogsSectionProps = {
  appLog: string[]
  mihomoLog: string[]
  loading: boolean
  onReload: () => void
}

function LogsSection({ appLog, mihomoLog, loading, onReload }: LogsSectionProps) {
  return (
    <section className="flex min-h-0 flex-1 flex-col rounded-lg border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface)] p-4">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold text-[color:var(--color-text-main)]">日志与监控</h3>
        <button
          type="button"
          className="rounded border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface-soft)] px-2 py-1 text-[11px] text-[color:var(--color-text-main)] hover:bg-[color:var(--color-accent)]"
          onClick={onReload}
        >
          刷新日志
        </button>
      </div>
      <p className="mt-1 text-xs text-[color:var(--color-text-muted)]">
        查看应用日志和 Mihomo 内核日志（最近若干行），用于排查下载、合并和运行时错误。
      </p>

      <div className="mt-3 grid min-h-0 flex-1 gap-3 md:grid-cols-2">
        <div className="flex min-h-0 flex-col">
          <div className="mb-1 flex items-center justify-between">
            <span className="text-xs font-medium text-[color:var(--color-text-main)]">
              应用日志 app.log
            </span>
          </div>
          <div className="flex-1 min-h-0 overflow-auto rounded border border-[color:var(--color-log-border)] bg-[color:var(--color-log-bg)] p-2">
            {loading ? (
              <p className="text-[11px] text-[color:var(--color-log-text-muted)]">正在加载日志…</p>
            ) : appLog.length === 0 ? (
              <p className="text-[11px] text-[color:var(--color-log-text-muted)]">
                暂无日志或日志文件未创建。
              </p>
            ) : (
              <pre className="whitespace-pre-wrap break-all font-mono text-[11px] text-[color:var(--color-log-text-main)]">
                <AnsiText text={appLog.join('\n')} />
              </pre>
            )}
          </div>
        </div>

        <div className="flex min-h-0 flex-col">
          <div className="mb-1 flex items-center justify-between">
            <span className="text-xs font-medium text-[color:var(--color-text-main)]">
              Mihomo 日志 mihomo.log
            </span>
          </div>
          <div className="flex-1 min-h-0 overflow-auto rounded border border-[color:var(--color-log-border)] bg-[color:var(--color-log-bg)] p-2">
            {loading ? (
              <p className="text-[11px] text-[color:var(--color-log-text-muted)]">正在加载日志…</p>
            ) : mihomoLog.length === 0 ? (
              <p className="text-[11px] text-[color:var(--color-log-text-muted)]">
                暂无日志，可能尚未启动内核或未产生输出。
              </p>
            ) : (
              <pre className="whitespace-pre-wrap break-all font-mono text-[11px] text-[color:var(--color-log-text-main)]">
                <AnsiText text={mihomoLog.join('\n')} />
              </pre>
            )}
          </div>
        </div>
      </div>
    </section>
  )
}

export default LogsSection
