import type { CoreStatus } from '../types'

type SystemStatusSectionProps = {
  coreStatus: CoreStatus | null
  subscriptionsCount: number
  passwordSet: boolean
}

function StatCard({
  label,
  value,
  tone = 'default',
}: {
  label: string
  value: string
  tone?: 'default' | 'success' | 'danger' | 'muted'
}) {
  const valueClass =
    tone === 'success'
      ? 'text-[color:var(--color-success)]'
      : tone === 'danger'
        ? 'text-[color:var(--color-danger)]'
        : tone === 'muted'
          ? 'text-[color:var(--color-text-muted)]'
          : 'text-[color:var(--color-text-main)]'

  return (
    <div className="ui-card px-7 py-8">
      <p className="text-lg text-[color:var(--color-text-muted)]">{label}</p>
      <p className={`text-2xl font-semibold mt-4 ${valueClass}`}>{value}</p>
    </div>
  )
}

function SystemStatusSection({
  coreStatus,
  subscriptionsCount,
  passwordSet,
}: SystemStatusSectionProps) {
  return (
    <section className="grid gap-5 sm:grid-cols-2 xl:grid-cols-3">
      <StatCard
        label="内核"
        value={coreStatus?.running ? '运行中' : '未运行'}
        tone={coreStatus?.running ? 'success' : 'muted'}
      />
      <StatCard label="订阅" value={String(subscriptionsCount)} />
      <StatCard
        label="安全"
        value={passwordSet ? '已保护' : '未设密'}
        tone={passwordSet ? 'success' : 'danger'}
      />
    </section>
  )
}

export default SystemStatusSection
