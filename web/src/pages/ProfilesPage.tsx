import { useEffect, useState } from 'react'
import { useUserProfiles } from '../hooks/useUserProfiles'
import { useMergedConfig } from '../hooks/useMergedConfig'
import UserProfilesSection from '../components/UserProfilesSection'

function ProfilesPage() {
  const {
    profiles,
    activeId,
    loading: profilesLoading,
    saving,
    creating,
    load,
    loadDetail,
    create,
    update,
    remove,
    activate,
  } = useUserProfiles()

  const {
    content: mergedConfig,
    loading: mergedLoading,
    load: loadMerged,
  } = useMergedConfig()

  const [editingId, setEditingId] = useState<string | null>(null)
  const [userProfileName, setUserProfileName] = useState('')
  const [userProfileContent, setUserProfileContent] = useState('')
  const [newUserProfileName, setNewUserProfileName] = useState('')

  useEffect(() => {
    void load()
    void loadMerged()
  }, [load, loadMerged])

  const handleLoadDetail = async (id: string) => {
    const detail = await loadDetail(id)
    if (!detail) return
    setEditingId(detail.id)
    setUserProfileName(detail.name)
    setUserProfileContent(detail.content)
  }

  const handleCreate = async () => {
    const created = await create(newUserProfileName, userProfileContent)
    if (!created) return
    setNewUserProfileName('')
    await handleLoadDetail(created.id)
    void loadMerged()
  }

  const handleSave = async () => {
    if (!editingId) {
      return
    }
    const detail = await update({
      id: editingId,
      name: userProfileName,
      content: userProfileContent,
    })
    if (!detail) return
    void loadMerged()
  }

  const handleDelete = async (id: string) => {
    if (!window.confirm('确认删除该用户 profile？')) return
    await remove(id)
    if (editingId === id) {
      setEditingId(null)
      setUserProfileName('')
      setUserProfileContent('')
    }
    void loadMerged()
  }

  const handleActivate = async (id: string) => {
    await activate(id)
    void loadMerged()
  }

  return (
    <UserProfilesSection
      userProfiles={profiles}
      userProfilesLoading={profilesLoading}
      activeUserProfileId={activeId}
      userProfileName={userProfileName}
      userProfileContent={userProfileContent}
      userProfileSaving={saving}
      newUserProfileName={newUserProfileName}
      creatingUserProfile={creating}
      mergedConfig={mergedConfig}
      mergedConfigLoading={mergedLoading}
      onReloadUserProfiles={() => {
        void load()
      }}
      onLoadUserProfileDetail={(id) => {
        void handleLoadDetail(id)
      }}
      onActivateUserProfile={(id) => {
        void handleActivate(id)
      }}
      onDeleteUserProfile={(id) => {
        void handleDelete(id)
      }}
      onNewUserProfileNameChange={setNewUserProfileName}
      onCreateUserProfile={() => {
        void handleCreate()
      }}
      onUserProfileNameChange={setUserProfileName}
      onUserProfileContentChange={setUserProfileContent}
      onSaveUserProfile={() => {
        void handleSave()
      }}
      onReloadMergedConfig={() => {
        void loadMerged()
      }}
    />
  )
}

export default ProfilesPage

