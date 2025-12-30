import type { ScheduledTaskConfig } from '../types'

type TaskProps = {
  title: string
  description: string
  value: ScheduledTaskConfig | null
  onChange: (value: ScheduledTaskConfig) => void
}

function TaskRow({ title, description, value, onChange }: TaskProps) {
  const cfg: ScheduledTaskConfig = value ?? {
    cron: '',
    enabled: false,
    last_run_time: null,
    last_run_status: null,
    last_run_message: null,
  }

  const handleToggle = (checked: boolean) => {
    onChange({ ...cfg, enabled: checked })
  }

  const handleCronChange = (cron: string) => {
    onChange({ ...cfg, cron })
  }

  let statusText = '尚未执行'
  let statusClass = 'text-[color:var(--color-text-muted)]'
  if (cfg.last_run_status === 'ok') {
    statusText = '最近执行：成功'
    statusClass = 'text-[color:var(--color-success)]'
  } else if (cfg.last_run_status === 'skipped') {
    statusText = cfg.last_run_message
      ? `最近执行：已跳过（${cfg.last_run_message}）`
      : '最近执行：已跳过'
    statusClass = 'text-[color:var(--color-text-muted)]'
  } else if (cfg.last_run_status === 'error') {
    statusText = cfg.last_run_message
      ? `最近执行：失败（${cfg.last_run_message}）`
      : '最近执行：失败'
    statusClass = 'text-[color:var(--color-danger)]'
  }

  return (
    <div className="rounded-md border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface-soft)] p-3">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="text-sm font-medium text-[color:var(--color-text-main)]">{title}</div>
          <p className="mt-0.5 text-xs text-[color:var(--color-text-muted)]">{description}</p>
        </div>
        <button
          type="button"
          onClick={() => {
            handleToggle(!cfg.enabled)
          }}
          className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs transition ${
            cfg.enabled
              ? 'border-[color:var(--color-success)] bg-[color:var(--color-success-soft)] text-[color:var(--color-success)]'
              : 'border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface)] text-[color:var(--color-text-muted)]'
          }`}
        >
          <span
            className={`mr-1 inline-block h-1.5 w-1.5 rounded-full ${
              cfg.enabled ? 'bg-[color:var(--color-success)]' : 'bg-[color:var(--color-border-subtle)]'
            }`}
          />
          {cfg.enabled ? '已启用' : '未启用'}
        </button>
      </div>
      <div className="mt-2 flex flex-col gap-1.5">
        <label className="text-xs text-[color:var(--color-text-muted)]">
          Cron 表达式（分钟 精确到 5 字段，如 <code>0 3 * * *</code>）
        </label>
        <input
          type="text"
          value={cfg.cron}
          onChange={(e) => {
            handleCronChange(e.target.value)
          }}
          placeholder="例如：0 3 * * *（每天 03:00），*/30 * * * *（每 30 分钟）"
          className="w-full rounded-md border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface-soft)] px-2 py-1 text-xs text-[color:var(--color-text-main)] outline-none ring-0 ring-[color:var(--color-success-soft)] focus:border-[color:var(--color-success)] focus:ring-1"
        />
        <p className={`mt-1 text-[11px] ${statusClass}`}>{statusText}</p>
      </div>
    </div>
  )
}

type SchedulerSectionProps = {
  subscriptionTask: ScheduledTaskConfig | null
  geoipTask: ScheduledTaskConfig | null
  onChangeSubscriptionTask: (value: ScheduledTaskConfig) => void
  onChangeGeoipTask: (value: ScheduledTaskConfig) => void
  onSave: () => void
  saving: boolean
}

function SchedulerSection({
  subscriptionTask,
  geoipTask,
  onChangeSubscriptionTask,
  onChangeGeoipTask,
  onSave,
  saving,
}: SchedulerSectionProps) {
  return (
    <section className="rounded-lg border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface)] p-4">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h2 className="text-base font-medium text-[color:var(--color-text-main)]">自动更新计划</h2>
          <p className="mt-1 text-xs text-[color:var(--color-text-muted)]">
            配置订阅与 GeoIP 数据库的自动更新计划，时间基于路由器本地时间。关闭开关则该任务不会被后台定时执行。
          </p>
        </div>
        <button
          type="button"
          onClick={onSave}
          disabled={saving}
          className="inline-flex items-center rounded-md border border-[color:var(--color-primary)] bg-[color:var(--color-primary-soft)] px-3 py-1 text-xs font-medium text-[color:var(--color-primary)] transition hover:bg-[color:var(--color-accent)] disabled:cursor-not-allowed disabled:border-[color:var(--color-border-subtle)] disabled:bg-[color:var(--color-surface-soft)] disabled:text-[color:var(--color-text-soft)]"
        >
          {saving ? '保存中…' : '保存设置'}
        </button>
      </div>
      <div className="mt-3 grid gap-3 md:grid-cols-2">
        <TaskRow
          title="订阅自动更新"
          description="定期拉取当前活跃订阅的配置，并自动合并到 merged.yaml。"
          value={subscriptionTask}
          onChange={onChangeSubscriptionTask}
        />
        <TaskRow
          title="GeoIP 数据库自动更新"
          description="从官方镜像自动下载最新 geoip.metadb，保存到 config 目录。"
          value={geoipTask}
          onChange={onChangeGeoipTask}
        />
      </div>
    </section>
  )
}

export default SchedulerSection
