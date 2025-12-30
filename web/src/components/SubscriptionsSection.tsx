import type { FormEvent } from 'react'
import type { Subscription } from '../types'

type SubscriptionsSectionProps = {
  subscriptions: Subscription[]
  loading: boolean
  saving: boolean
  editingId: string | null
  name: string
  url: string
  onChangeName: (value: string) => void
  onChangeUrl: (value: string) => void
  onResetForm: () => void
  onSubmit: (event: FormEvent<HTMLFormElement>) => void
  onReload: () => void
  onEdit: (subscription: Subscription) => void
  onDelete: (id: string) => void
  onActivate: (id: string) => void
  onFetch: (id: string) => void
}

function activeLabel(sub: Subscription) {
  if (sub.is_active) return '当前订阅'
  return '设为当前'
}

function SubscriptionsSection({
  subscriptions,
  loading,
  saving,
  editingId,
  name,
  url,
  onChangeName,
  onChangeUrl,
  onResetForm,
  onSubmit,
  onReload,
  onEdit,
  onDelete,
  onActivate,
  onFetch,
}: SubscriptionsSectionProps) {
  return (
    <section className="rounded-lg border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface)] p-4">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-base font-medium text-[color:var(--color-text-main)]">订阅管理</h2>
        <button
          type="button"
          className="rounded border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface-soft)] px-3 py-1 text-xs text-[color:var(--color-text-main)] hover:bg-[color:var(--color-accent)]"
          onClick={onReload}
        >
          刷新
        </button>
      </div>

      <div className="mt-4 grid gap-6 md:grid-cols-[18rem_1fr]">
        <form onSubmit={onSubmit} className="space-y-3">
          <div>
            <label className="block text-xs font-medium text-[color:var(--color-text-main)]">
              订阅名称
            </label>
            <input
              className="mt-1 w-full rounded border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface-soft)] px-3 py-1.5 text-sm text-[color:var(--color-text-main)] outline-none ring-0 focus:border-[color:var(--color-primary)]"
              value={name}
              onChange={(e) => onChangeName(e.target.value)}
              placeholder="例如：主订阅"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-[color:var(--color-text-main)]">
              订阅 URL
            </label>
            <input
              className="mt-1 w-full rounded border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface-soft)] px-3 py-1.5 text-sm text-[color:var(--color-text-main)] outline-none ring-0 focus:border-[color:var(--color-primary)]"
              value={url}
              onChange={(e) => onChangeUrl(e.target.value)}
              placeholder="例如：https://example.com/subscription"
            />
          </div>

          <div className="flex items-center gap-2 pt-1">
            <button
              type="submit"
              disabled={saving}
              className="whitespace-nowrap rounded bg-[color:var(--color-primary)] px-3 py-1.5 text-xs font-medium text-[color:var(--color-primary-on)] hover:bg-[#6b6f63] disabled:cursor-not-allowed disabled:opacity-60"
            >
              {editingId ? '保存修改' : '新增订阅'}
            </button>
            {editingId && (
              <button
                type="button"
                className="rounded border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface-soft)] px-3 py-1.5 text-xs text-[color:var(--color-text-main)] hover:bg-[color:var(--color-accent)]"
                onClick={onResetForm}
              >
                取消编辑
              </button>
            )}
          </div>
        </form>

        <div className="min-w-0 space-y-2">
          {loading ? (
            <p className="text-xs text-[color:var(--color-text-muted)]">正在加载订阅列表…</p>
          ) : subscriptions.length === 0 ? (
            <p className="text-xs text-[color:var(--color-text-soft)]">
              暂无订阅，请在左侧添加新的订阅。
            </p>
          ) : (
            <ul className="space-y-2">
              {subscriptions.map((sub) => (
                <li
                  key={sub.id}
                  className="flex flex-col gap-2 rounded border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface-soft)] px-3 py-2 text-xs md:flex-row md:items-center md:justify-between"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate font-semibold text-[color:var(--color-text-main)]">
                        {sub.name}
                      </span>
                      {sub.is_active && (
                        <span className="rounded-full bg-[color:var(--color-success-soft)] px-2 py-0.5 text-[10px] font-medium text-[color:var(--color-success)]">
                          当前
                        </span>
                      )}
                    </div>
                    <p className="mt-0.5 truncate text-[11px] text-[color:var(--color-text-muted)]">
                      {sub.url}
                    </p>
                    {sub.last_fetch_time && (
                      <p className="mt-0.5 text-[11px] text-[color:var(--color-text-soft)]">
                        最近拉取：{sub.last_fetch_time}（状态：
                        {sub.last_fetch_status ?? '未知'}）
                      </p>
                    )}
                  </div>

                  <div className="flex flex-wrap items-center gap-1 md:flex-nowrap">
                    <button
                      type="button"
                      className="rounded border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface-soft)] px-2 py-1 text-[11px] text-[color:var(--color-text-main)] hover:bg-[color:var(--color-accent)]"
                      onClick={() => onEdit(sub)}
                    >
                      编辑
                    </button>
                    <button
                      type="button"
                      className="rounded border border-[color:var(--color-danger)] bg-[color:var(--color-danger-soft)] px-2 py-1 text-[11px] text-[color:var(--color-danger)] hover:bg-[#f0c7bc]"
                      onClick={() => onDelete(sub.id)}
                    >
                      删除
                    </button>
                    <button
                      type="button"
                      className="rounded border border-[color:var(--color-success)] bg-[color:var(--color-success-soft)] px-2 py-1 text-[11px] text-[color:var(--color-success)] hover:bg-[#c6d7bd] disabled:cursor-default disabled:opacity-60"
                      disabled={sub.is_active}
                      onClick={() => onActivate(sub.id)}
                    >
                      {activeLabel(sub)}
                    </button>
                    <button
                      type="button"
                      className="rounded border border-[color:var(--color-primary)] bg-[color:var(--color-surface-soft)] px-2 py-1 text-[11px] text-[color:var(--color-primary)] hover:bg-[color:var(--color-accent)]"
                      onClick={() => onFetch(sub.id)}
                    >
                      拉取
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </section>
  )
}

export default SubscriptionsSection
