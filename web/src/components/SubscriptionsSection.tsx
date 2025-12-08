import type { FormEvent } from 'react'
import type { Subscription } from '../types'

type SubscriptionsSectionProps = {
  subscriptions: Subscription[]
  loading: boolean
  saving: boolean
  editingId: string | null
  name: string
  url: string
  message: string | null
  error: string | null
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
  message,
  error,
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
    <section className="rounded-lg border border-slate-800 bg-slate-900/60 p-4">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-base font-medium text-slate-100">订阅管理</h2>
        <button
          type="button"
          className="rounded border border-slate-700 bg-slate-900 px-3 py-1 text-xs text-slate-200 hover:bg-slate-800"
          onClick={onReload}
        >
          刷新
        </button>
      </div>

      {message && (
        <p className="mt-2 text-xs text-emerald-400" role="status">
          {message}
        </p>
      )}
      {error && (
        <p className="mt-2 text-xs text-red-400 break-words" role="alert">
          {error}
        </p>
      )}

      <div className="mt-4 grid gap-6 md:grid-cols-[18rem_1fr]">
        <form onSubmit={onSubmit} className="space-y-3">
          <div>
            <label className="block text-xs font-medium text-slate-300">
              订阅名称
            </label>
            <input
              className="mt-1 w-full rounded border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm text-slate-100 outline-none ring-0 focus:border-sky-500"
              value={name}
              onChange={(e) => onChangeName(e.target.value)}
              placeholder="例如：主订阅"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-slate-300">
              订阅 URL
            </label>
            <input
              className="mt-1 w-full rounded border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm text-slate-100 outline-none ring-0 focus:border-sky-500"
              value={url}
              onChange={(e) => onChangeUrl(e.target.value)}
              placeholder="例如：https://example.com/subscription"
            />
          </div>

          <div className="flex items-center gap-2 pt-1">
            <button
              type="submit"
              disabled={saving}
              className="whitespace-nowrap rounded bg-sky-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
            >
              {editingId ? '保存修改' : '新增订阅'}
            </button>
            {editingId && (
              <button
                type="button"
                className="rounded border border-slate-700 bg-slate-900 px-3 py-1.5 text-xs text-slate-200 hover:bg-slate-800"
                onClick={onResetForm}
              >
                取消编辑
              </button>
            )}
          </div>
        </form>

        <div className="min-w-0 space-y-2">
          {loading ? (
            <p className="text-xs text-slate-400">正在加载订阅列表…</p>
          ) : subscriptions.length === 0 ? (
            <p className="text-xs text-slate-400">
              暂无订阅，请在左侧添加新的订阅。
            </p>
          ) : (
            <ul className="space-y-2">
              {subscriptions.map((sub) => (
                <li
                  key={sub.id}
                  className="flex flex-col gap-2 rounded border border-slate-800 bg-slate-900/60 px-3 py-2 text-xs md:flex-row md:items-center md:justify-between"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate font-semibold text-slate-100">
                        {sub.name}
                      </span>
                      {sub.is_active && (
                        <span className="rounded-full bg-emerald-500/15 px-2 py-0.5 text-[10px] font-medium text-emerald-400">
                          当前
                        </span>
                      )}
                    </div>
                    <p className="mt-0.5 truncate text-[11px] text-slate-400">
                      {sub.url}
                    </p>
                    {sub.last_fetch_time && (
                      <p className="mt-0.5 text-[11px] text-slate-500">
                        最近拉取：{sub.last_fetch_time}（状态：
                        {sub.last_fetch_status ?? '未知'}）
                      </p>
                    )}
                  </div>

                  <div className="flex flex-wrap items-center gap-1 md:flex-nowrap">
                    <button
                      type="button"
                      className="rounded border border-slate-700 bg-slate-900 px-2 py-1 text-[11px] text-slate-200 hover:bg-slate-800"
                      onClick={() => onEdit(sub)}
                    >
                      编辑
                    </button>
                    <button
                      type="button"
                      className="rounded border border-red-700 bg-slate-900 px-2 py-1 text-[11px] text-red-300 hover:bg-red-800/40"
                      onClick={() => onDelete(sub.id)}
                    >
                      删除
                    </button>
                    <button
                      type="button"
                      className="rounded border border-emerald-700 bg-slate-900 px-2 py-1 text-[11px] text-emerald-300 hover:bg-emerald-800/40 disabled:cursor-default disabled:opacity-60"
                      disabled={sub.is_active}
                      onClick={() => onActivate(sub.id)}
                    >
                      {activeLabel(sub)}
                    </button>
                    <button
                      type="button"
                      className="rounded border border-sky-700 bg-slate-900 px-2 py-1 text-[11px] text-sky-300 hover:bg-sky-800/40"
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

