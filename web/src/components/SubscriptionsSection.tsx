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
    <section className="space-y-5">
      <form onSubmit={onSubmit} className="ui-card p-8">
        <div className="flex items-center justify-between gap-4">
          <h2 className="text-lg font-semibold">
            {editingId ? '编辑' : '新增'}
          </h2>
          <button type="button" className="ui-btn-default" onClick={onReload}>
            刷新
          </button>
        </div>

        <div className="mt-6 grid gap-5 md:grid-cols-2">
          <div>
            <label className="text-lg text-[color:var(--color-text-muted)] block">名称</label>
            <input
              className="ui-input mt-2"
              value={name}
              onChange={(e) => onChangeName(e.target.value)}
              placeholder="主订阅"
            />
          </div>
          <div>
            <label className="text-lg text-[color:var(--color-text-muted)] block">地址</label>
            <input
              className="ui-input mt-2"
              value={url}
              onChange={(e) => onChangeUrl(e.target.value)}
              placeholder="https://"
            />
          </div>
        </div>

        <div className="mt-6 flex items-center gap-3">
          <button type="submit" disabled={saving} className="ui-btn-primary">
            {editingId ? '保存' : '添加'}
          </button>
          {editingId && (
            <button type="button" className="ui-btn-default" onClick={onResetForm}>
              取消
            </button>
          )}
        </div>
      </form>

      <div className="ui-card overflow-hidden p-4">
        {loading ? (
          <p className="text-lg text-[color:var(--color-text-muted)] px-4 py-8">加载中…</p>
        ) : subscriptions.length === 0 ? (
          <p className="text-lg text-[color:var(--color-text-muted)] px-4 py-8">暂无订阅</p>
        ) : (
          <div className="overflow-x-auto">
            <table className="text-lg min-w-full">
              <thead className="text-lg text-[color:var(--color-text-muted)] text-left">
                <tr>
                  <th className="px-4 py-4 font-medium">名称</th>
                  <th className="px-4 py-4 font-medium">地址</th>
                  <th className="px-4 py-4 text-right font-medium">操作</th>
                </tr>
              </thead>
              <tbody>
                {subscriptions.map((sub) => (
                  <tr key={sub.id} className="even:bg-[color:var(--color-surface-soft)]">
                    <td className="px-4 py-5">
                      <div className="flex items-center gap-2">
                        <span className="font-medium">{sub.name}</span>
                        {sub.is_active && (
                          <span className="inline-flex items-center rounded-full px-3 py-1 text-lg bg-[color:var(--color-success-soft)] text-[color:var(--color-success)]">
                            当前
                          </span>
                        )}
                      </div>
                    </td>
                    <td className="max-w-[22rem] px-4 py-5">
                      <span className="block truncate text-[color:var(--color-text-muted)]">
                        {sub.url}
                      </span>
                    </td>
                    <td className="px-4 py-5">
                      <div className="flex flex-wrap justify-end gap-2">
                        <button type="button" className="ui-btn-default" onClick={() => onEdit(sub)}>
                          编辑
                        </button>
                        <button type="button" className="ui-btn-danger" onClick={() => onDelete(sub.id)}>
                          删除
                        </button>
                        <button
                          type="button"
                          className="ui-btn-success"
                          disabled={sub.is_active}
                          onClick={() => onActivate(sub.id)}
                        >
                          {sub.is_active ? '当前' : '启用'}
                        </button>
                        <button type="button" className="ui-btn-default" onClick={() => onFetch(sub.id)}>
                          拉取
                        </button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </section>
  )
}

export default SubscriptionsSection
