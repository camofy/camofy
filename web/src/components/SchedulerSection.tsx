import type { ScheduledTaskConfig } from '../types'

type TaskProps = {
  title: string
  value: ScheduledTaskConfig | null
  onChange: (value: ScheduledTaskConfig) => void
}

function TaskRow({ title, value, onChange }: TaskProps) {
  const cfg: ScheduledTaskConfig = value ?? {
    cron: '',
    enabled: false,
    last_run_time: null,
    last_run_status: null,
    last_run_message: null,
  }

  let statusText = '尚未执行'
  let statusClass = 'text-[color:var(--color-text-muted)]'
  if (cfg.last_run_status === 'ok') {
    statusText = '成功'
    statusClass = 'text-[color:var(--color-success)]'
  } else if (cfg.last_run_status === 'skipped') {
    statusText = '已跳过'
    statusClass = 'text-[color:var(--color-text-muted)]'
  } else if (cfg.last_run_status === 'error') {
    statusText = '失败'
    statusClass = 'text-[color:var(--color-danger)]'
  }

  return (
    <div className="rounded-[24px] bg-[color:var(--color-surface-soft)] p-6">
      <div className="flex items-center justify-between gap-4">
        <div className="text-lg font-medium">{title}</div>
        <button
          type="button"
          onClick={() => onChange({ ...cfg, enabled: !cfg.enabled })}
          className={`inline-flex items-center rounded-full px-3 py-1 text-lg ${
            cfg.enabled
              ? 'bg-[color:var(--color-success-soft)] text-[color:var(--color-success)]'
              : 'bg-white text-[color:var(--color-text-muted)]'
          }`}
        >
          {cfg.enabled ? '已启用' : '未启用'}
        </button>
      </div>
      <input
        type="text"
        value={cfg.cron}
        onChange={(e) => onChange({ ...cfg, cron: e.target.value })}
        placeholder="0 3 * * *"
        className="ui-input mt-5"
      />
      <p className={`mt-3 text-lg ${statusClass}`}>{statusText}</p>
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
    <section className="ui-card p-8">
      <div className="flex items-center justify-between gap-4">
        <h2 className="text-lg font-semibold">计划</h2>
        <button type="button" onClick={onSave} disabled={saving} className="ui-btn-primary">
          {saving ? '保存中' : '保存'}
        </button>
      </div>
      <div className="mt-6 grid gap-5 md:grid-cols-2">
        <TaskRow title="订阅更新" value={subscriptionTask} onChange={onChangeSubscriptionTask} />
        <TaskRow title="GeoIP 更新" value={geoipTask} onChange={onChangeGeoipTask} />
      </div>
    </section>
  )
}

export default SchedulerSection
