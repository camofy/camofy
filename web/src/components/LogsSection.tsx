import AnsiText from './AnsiText'

type LogsSectionProps = {
  appLog: string[]
  mihomoLog: string[]
  loading: boolean
  onReload: () => void
}

function LogsSection({ appLog, mihomoLog, loading, onReload }: LogsSectionProps) {
  return (
    <section className="ui-card flex min-h-0 flex-1 flex-col p-8">
      <div className="flex items-center justify-between gap-4">
        <h3 className="sr-only">日志</h3>
        <button type="button" className="ui-btn-default" onClick={onReload}>
          刷新
        </button>
      </div>

      <div className="mt-6 grid min-h-0 flex-1 gap-5 md:grid-cols-2">
        <div className="flex min-h-0 flex-col">
          <div className="text-lg text-[color:var(--color-text-muted)] mb-3">应用</div>
          <div className="min-h-[16rem] flex-1 overflow-auto rounded-[24px] bg-[color:var(--color-log-bg)] p-5">
            {loading ? (
              <p className="log-pre text-[color:var(--color-log-text-muted)]">加载中…</p>
            ) : appLog.length === 0 ? (
              <p className="log-pre text-[color:var(--color-log-text-muted)]">暂无日志</p>
            ) : (
              <pre className="log-pre whitespace-pre-wrap break-all text-[color:var(--color-log-text-main)]">
                <AnsiText text={appLog.join('\n')} />
              </pre>
            )}
          </div>
        </div>

        <div className="flex min-h-0 flex-col">
          <div className="text-lg text-[color:var(--color-text-muted)] mb-3">内核</div>
          <div className="min-h-[16rem] flex-1 overflow-auto rounded-[24px] bg-[color:var(--color-log-bg)] p-5">
            {loading ? (
              <p className="log-pre text-[color:var(--color-log-text-muted)]">加载中…</p>
            ) : mihomoLog.length === 0 ? (
              <p className="log-pre text-[color:var(--color-log-text-muted)]">暂无日志</p>
            ) : (
              <pre className="log-pre whitespace-pre-wrap break-all text-[color:var(--color-log-text-main)]">
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
