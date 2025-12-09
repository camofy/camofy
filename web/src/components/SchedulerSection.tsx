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
  let statusClass = 'text-slate-400'
  if (cfg.last_run_status === 'ok') {
    statusText = '最近执行：成功'
    statusClass = 'text-emerald-400'
  } else if (cfg.last_run_status === 'skipped') {
    statusText = cfg.last_run_message
      ? `最近执行：已跳过（${cfg.last_run_message}）`
      : '最近执行：已跳过'
    statusClass = 'text-slate-400'
  } else if (cfg.last_run_status === 'error') {
    statusText = cfg.last_run_message
      ? `最近执行：失败（${cfg.last_run_message}）`
      : '最近执行：失败'
    statusClass = 'text-rose-400'
  }

  return (
    <div className="rounded-md border border-slate-800/80 bg-slate-900/60 p-3">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="text-sm font-medium text-slate-100">{title}</div>
          <p className="mt-0.5 text-xs text-slate-400">{description}</p>
        </div>
        <button
          type="button"
          onClick={() => {
            handleToggle(!cfg.enabled)
          }}
          className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs transition ${
            cfg.enabled
              ? 'border-emerald-500/60 bg-emerald-500/10 text-emerald-300'
              : 'border-slate-700 bg-slate-800 text-slate-300'
          }`}
        >
          <span
            className={`mr-1 inline-block h-1.5 w-1.5 rounded-full ${
              cfg.enabled ? 'bg-emerald-400' : 'bg-slate-500'
            }`}
          />
          {cfg.enabled ? '已启用' : '未启用'}
        </button>
      </div>
      <div className="mt-2 flex flex-col gap-1.5">
        <label className="text-xs text-slate-400">
          Cron 表达式（分钟 精确到 5 字段，如 <code>0 3 * * *</code>）
        </label>
        <input
          type="text"
          value={cfg.cron}
          onChange={(e) => {
            handleCronChange(e.target.value)
          }}
          placeholder="例如：0 3 * * *（每天 03:00），*/30 * * * *（每 30 分钟）"
          className="w-full rounded-md border border-slate-700 bg-slate-950 px-2 py-1 text-xs text-slate-100 outline-none ring-0 ring-emerald-500/50 focus:border-emerald-500 focus:ring-1"
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
    <section className="rounded-lg border border-slate-800 bg-slate-900/60 p-4">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h2 className="text-base font-medium text-slate-100">自动更新计划</h2>
          <p className="mt-1 text-xs text-slate-400">
            配置订阅与 GeoIP 数据库的自动更新计划，时间基于路由器本地时间。关闭开关则该任务不会被后台定时执行。
          </p>
        </div>
        <button
          type="button"
          onClick={onSave}
          disabled={saving}
          className="inline-flex items-center rounded-md border border-emerald-500/70 bg-emerald-500/10 px-3 py-1 text-xs font-medium text-emerald-200 transition hover:bg-emerald-500/20 disabled:cursor-not-allowed disabled:border-slate-700 disabled:bg-slate-800 disabled:text-slate-400"
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

