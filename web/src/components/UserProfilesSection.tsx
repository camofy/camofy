import type { UserProfileSummary } from '../types'
import { formatTime } from '../format'
import VirtualPre from './VirtualPre'

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
    <div className="ui-card flex min-h-0 flex-1 flex-col p-8">
      <div className="grid min-h-0 flex-1 gap-6 lg:grid-cols-[20rem_1fr]">
        <div className="flex min-h-0 flex-col gap-5">
          <div>
            <label className="text-lg text-[color:var(--color-text-muted)] block">新建</label>
            <div className="mt-2 flex items-center gap-3">
              <input
                className="ui-input"
                value={newUserProfileName}
                onChange={(e) => onNewUserProfileNameChange(e.target.value)}
                placeholder="名称"
              />
              <button
                type="button"
                disabled={creatingUserProfile}
                onClick={onCreateUserProfile}
                className="ui-btn-primary shrink-0"
              >
                创建
              </button>
            </div>
          </div>

          <div className="flex min-h-0 flex-1 flex-col">
            <div className="mb-3 flex items-center justify-between">
              <span className="text-lg text-[color:var(--color-text-muted)]">列表</span>
              <button type="button" className="ui-btn-default" onClick={onReloadUserProfiles}>
                刷新
              </button>
            </div>
            <div className="min-h-[8rem] flex-1 overflow-auto rounded-[24px] bg-[color:var(--color-surface-soft)] p-3">
              {userProfilesLoading ? (
                <p className="text-lg text-[color:var(--color-text-muted)] p-3">加载中…</p>
              ) : userProfiles.length === 0 ? (
                <p className="text-lg text-[color:var(--color-text-muted)] p-3">暂无配置</p>
              ) : (
                <ul className="space-y-3">
                  {userProfiles.map((p) => (
                    <li key={p.id} className="rounded-2xl bg-white px-4 py-4">
                      <div className="flex items-center gap-2">
                        <span className="text-lg truncate font-medium">{p.name}</span>
                        {p.is_active && (
                          <span className="inline-flex items-center rounded-full px-3 py-1 text-lg bg-[color:var(--color-success-soft)] text-[color:var(--color-success)]">
                            当前
                          </span>
                        )}
                      </div>
                      {p.last_modified_time && (
                        <p className="text-lg text-[color:var(--color-text-muted)] mt-1">
                          {formatTime(p.last_modified_time)}
                        </p>
                      )}
                      <div className="mt-3 flex flex-wrap gap-2">
                        <button
                          type="button"
                          className="ui-btn-default"
                          onClick={() => onLoadUserProfileDetail(p.id)}
                        >
                          编辑
                        </button>
                        <button
                          type="button"
                          className="ui-btn-success"
                          disabled={p.is_active}
                          onClick={() => onActivateUserProfile(p.id)}
                        >
                          {p.is_active ? '当前' : '启用'}
                        </button>
                        <button
                          type="button"
                          className="ui-btn-danger"
                          onClick={() => onDeleteUserProfile(p.id)}
                        >
                          删除
                        </button>
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </div>
        </div>

        <div className="flex min-h-0 flex-col gap-5">
          <div>
            <label className="text-lg text-[color:var(--color-text-muted)] block">名称</label>
            <input
              className="ui-input mt-2"
              value={userProfileName}
              onChange={(e) => onUserProfileNameChange(e.target.value)}
              placeholder="选择或新建"
            />
          </div>
          <div className="flex min-h-[8rem] flex-1 flex-col">
            <label className="text-lg text-[color:var(--color-text-muted)] block">内容</label>
            <textarea
              className="ui-input log-pre mt-2 min-h-[6rem] flex-1 resize-none overflow-auto"
              value={userProfileContent}
              onChange={(e) => onUserProfileContentChange(e.target.value)}
              placeholder=""
            />
          </div>
          <button
            type="button"
            disabled={userProfileSaving}
            onClick={onSaveUserProfile}
            className="ui-btn-primary self-start"
          >
            保存
          </button>
          <div className="flex min-h-0 flex-1 flex-col">
            <div className="mb-3 flex items-center justify-between">
              <span className="text-lg text-[color:var(--color-text-muted)]">预览</span>
              <button type="button" className="ui-btn-default" onClick={onReloadMergedConfig}>
                刷新
              </button>
            </div>
            <div className="min-h-[8rem] flex-1 overflow-hidden rounded-[24px] bg-[color:var(--color-log-bg)] p-5">
              {mergedConfigLoading ? (
                <p className="log-pre text-[color:var(--color-log-text-muted)]">加载中…</p>
              ) : (
                <VirtualPre
                  text={mergedConfig}
                  className="log-pre text-[color:var(--color-log-text-main)]"
                />
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

export default UserProfilesSection
