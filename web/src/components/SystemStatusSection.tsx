import type { CoreStatus } from '../types'

type SystemStatusSectionProps = {
  coreStatus: CoreStatus | null
  subscriptionsCount: number
  passwordSet: boolean
}

function SystemStatusSection({
  coreStatus,
  subscriptionsCount,
  passwordSet,
}: SystemStatusSectionProps) {
  return (
    <section className="rounded-lg border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface)] p-4">
      <h2 className="text-base font-medium text-[color:var(--color-text-main)]">系统状态</h2>
      <p className="mt-1 text-sm text-[color:var(--color-text-muted)]">
        后端 API 已提供{' '}
        <code className="rounded bg-[color:var(--color-surface-soft)] px-1 text-[color:var(--color-text-main)]">
          /api/health
        </code>{' '}
        健康检查接口，目前会在这里汇总 Mihomo 与配置的基础状态。
      </p>
      <div className="mt-3 space-y-1 text-xs text-[color:var(--color-text-main)]">
        <p>
          内核状态：{' '}
          {coreStatus?.running ? (
            <span className="text-[color:var(--color-success)]">
              运行中{coreStatus.pid ? `（PID：${coreStatus.pid}）` : ''}
            </span>
          ) : (
            <span className="text-[color:var(--color-text-muted)]">未运行</span>
          )}
        </p>
        <p>
          订阅数量： <span className="font-mono">{subscriptionsCount}</span>
        </p>
        <p>
          安全状态：{' '}
          {passwordSet ? (
            <span className="text-[color:var(--color-success)]">已设置访问密码</span>
          ) : (
            <span className="text-[color:var(--color-danger)]">
              未设置访问密码（建议尽快通过 API 或配置文件设置）
            </span>
          )}
        </p>
      </div>
    </section>
  )
}

export default SystemStatusSection
