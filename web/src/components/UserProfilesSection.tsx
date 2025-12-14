import type { UserProfileSummary } from '../types'

type UserProfilesSectionProps = {
  userProfiles: UserProfileSummary[]
  userProfilesLoading: boolean
  activeUserProfileId: string | null
  userProfileName: string
  userProfileContent: string
  userProfileSaving: boolean
  newUserProfileName: string
  creatingUserProfile: boolean
  mergedConfig: string
  mergedConfigLoading: boolean
  onReloadUserProfiles: () => void
  onLoadUserProfileDetail: (id: string) => void
  onActivateUserProfile: (id: string) => void
  onDeleteUserProfile: (id: string) => void
  onNewUserProfileNameChange: (value: string) => void
  onCreateUserProfile: () => void
  onUserProfileNameChange: (value: string) => void
  onUserProfileContentChange: (value: string) => void
  onSaveUserProfile: () => void
  onReloadMergedConfig: () => void
}

function UserProfilesSection({
  userProfiles,
  userProfilesLoading,
  activeUserProfileId,
  userProfileName,
  userProfileContent,
  userProfileSaving,
  newUserProfileName,
  creatingUserProfile,
  mergedConfig,
  mergedConfigLoading,
  onReloadUserProfiles,
  onLoadUserProfileDetail,
  onActivateUserProfile,
  onDeleteUserProfile,
  onNewUserProfileNameChange,
  onCreateUserProfile,
  onUserProfileNameChange,
  onUserProfileContentChange,
  onSaveUserProfile,
  onReloadMergedConfig,
}: UserProfilesSectionProps) {
  return (
    <div className="flex min-h-0 flex-1 flex-col rounded-lg border border-slate-800 bg-slate-900/60 p-4">
      <h3 className="text-sm font-semibold text-slate-100">
        配置管理（订阅配置 + 用户配置）
      </h3>
      <p className="mt-1 text-xs text-slate-400">
        在订阅基础上编写用户配置，并合并生成最终的{' '}
        <code className="rounded bg-slate-800 px-1 text-[11px]">merged.yaml</code>。
      </p>

      <div className="mt-3 grid min-h-0 flex-1 gap-4 md:grid-cols-[15rem_1fr]">
        <div className="flex min-h-0 flex-col space-y-3">
          <div>
            <label className="block text-xs font-medium text-slate-300">
              新建用户 profile
            </label>
            <div className="mt-1 flex items-center gap-2">
              <input
                className="w-full rounded border border-slate-700 bg-slate-900 px-2 py-1 text-xs text-slate-100 outline-none ring-0 focus:border-sky-500"
                value={newUserProfileName}
                onChange={(e) => onNewUserProfileNameChange(e.target.value)}
                placeholder="例如：路由器本地配置"
              />
              <button
                type="button"
                disabled={creatingUserProfile}
                onClick={onCreateUserProfile}
                className="whitespace-nowrap rounded bg-sky-600 px-2 py-1 text-[11px] font-medium text-white hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
              >
                创建
              </button>
            </div>
          </div>

          <div>
            <div className="mb-1 flex items-center justify-between">
              <span className="text-xs font-medium text-slate-300">
                用户 profile 列表
              </span>
              <button
                type="button"
                className="rounded border border-slate-700 bg-slate-900 px-2 py-0.5 text-[11px] text-slate-200 hover:bg-slate-800"
                onClick={onReloadUserProfiles}
              >
                刷新
              </button>
            </div>
            <div className="min-h-[3rem] flex-1 min-h-0 overflow-auto rounded border border-slate-800 bg-slate-950/60 p-2">
              {userProfilesLoading ? (
                <p className="text-[11px] text-slate-400">
                  正在加载用户配置列表…
                </p>
              ) : userProfiles.length === 0 ? (
                <p className="text-[11px] text-slate-500">
                  暂无用户 profile，请先在上方创建。
                </p>
              ) : (
                <ul className="space-y-1">
                  {userProfiles.map((p) => (
                    <li
                      key={p.id}
                      className="flex items-center justify-between gap-1 rounded border border-slate-800 bg-slate-900/60 px-2 py-1 text-[11px]"
                    >
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-1">
                          <span className="truncate">{p.name}</span>
                          {p.is_active && (
                            <span className="rounded bg-emerald-700/30 px-1 text-[10px] text-emerald-300">
                              当前
                            </span>
                          )}
                        </div>
                        {p.last_modified_time && (
                          <p className="truncate text-[10px] text-slate-500">
                            更新：{p.last_modified_time}
                          </p>
                        )}
                      </div>
                      <div className="flex flex-shrink-0 items-center gap-1">
                        <button
                          type="button"
                          className="rounded border border-slate-700 bg-slate-900 px-1.5 py-0.5 text-[10px] text-slate-200 hover:bg-slate-800"
                          onClick={() => onLoadUserProfileDetail(p.id)}
                        >
                          编辑
                        </button>
                        <button
                          type="button"
                          className="rounded border border-emerald-700 bg-slate-900 px-1.5 py-0.5 text-[10px] text-emerald-300 hover:bg-emerald-800/40 disabled:cursor-not-allowed disabled:opacity-60"
                          disabled={p.is_active}
                          onClick={() => onActivateUserProfile(p.id)}
                        >
                          {p.is_active ? '当前' : '设为当前'}
                        </button>
                        <button
                          type="button"
                          className="rounded border border-red-700 bg-slate-900 px-1.5 py-0.5 text-[10px] text-red-300 hover:bg-red-800/40"
                          onClick={() => onDeleteUserProfile(p.id)}
                        >
                          删
                        </button>
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </div>
        </div>

        <div className="flex min-h-0 flex-col space-y-3">
          <div className="flex-shrink-0">
            <label className="block text-xs font-medium text-slate-300">
              当前编辑的用户 profile
            </label>
            <input
              className="mt-1 w-full rounded border border-slate-700 bg-slate-900 px-3 py-1.5 text-xs text-slate-100 outline-none ring-0 focus:border-sky-500"
              value={userProfileName}
              onChange={(e) => onUserProfileNameChange(e.target.value)}
              placeholder="先在左侧选择或新建一个用户 profile"
            />
          </div>
          <div className="flex min-h-[8rem] flex-1 flex-col">
            <label className="block text-xs font-medium text-slate-300">
              用户 YAML 配置
            </label>
            <textarea
              className="mt-1 flex-1 min-h-[6rem] w-full resize-none overflow-auto rounded border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-[11px] text-slate-100 outline-none ring-0 focus:border-sky-500"
              value={userProfileContent}
              onChange={(e) => onUserProfileContentChange(e.target.value)}
              placeholder="# 在这里编写用户配置，支持 prepend-rules / append-rules / prepend-proxies / append-proxies 等字段"
            />
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              disabled={userProfileSaving}
              onClick={onSaveUserProfile}
              className="rounded bg-emerald-600 px-3 py-1.5 text-[11px] font-medium text-white hover:bg-emerald-500 disabled:cursor-not-allowed disabled:opacity-60"
            >
              保存并合并
            </button>
            {activeUserProfileId && (
              <span className="text-[11px] text-slate-500">
                当前活跃用户 profile ID：{activeUserProfileId}
              </span>
            )}
          </div>
          <div className="flex min-h-0 flex-1 flex-col">
            <div className="mb-1 flex items-center justify-between">
              <span className="text-xs font-medium text-slate-300">
                合并后的 merged.yaml 预览
              </span>
              <button
                type="button"
                className="rounded border border-slate-700 bg-slate-900 px-2 py-0.5 text-[11px] text-slate-200 hover:bg-slate-800"
                onClick={onReloadMergedConfig}
              >
                刷新
              </button>
            </div>
            <div className="flex-1 min-h-0 overflow-auto rounded border border-slate-800 bg-slate-950/60 p-2">
              {mergedConfigLoading ? (
                <p className="text-[11px] text-slate-400">
                  正在加载 merged.yaml…
                </p>
              ) : (
                <pre className="whitespace-pre-wrap break-all font-mono text-[11px] text-slate-200">
                  {mergedConfig}
                </pre>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

export default UserProfilesSection
