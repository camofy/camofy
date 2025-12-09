import { type FormEvent, useEffect, useState } from 'react'
import { useSubscriptions } from '../hooks/useSubscriptions'
import SubscriptionsSection from '../components/SubscriptionsSection'

function SubscriptionsPage() {
  const {
    subscriptions,
    loading,
    saving,
    load,
    save,
    remove,
    activate,
    fetchRemote,
  } = useSubscriptions()

  const [editingId, setEditingId] = useState<string | null>(null)
  const [name, setName] = useState('')
  const [url, setUrl] = useState('')

  useEffect(() => {
    void load()
  }, [load])

  const resetForm = () => {
    setEditingId(null)
    setName('')
    setUrl('')
  }

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault()
    await save({ id: editingId, name, url })
    if (!editingId) {
      resetForm()
    }
  }

  const handleEdit = (subId: string, subName: string, subUrl: string) => {
    setEditingId(subId)
    setName(subName)
    setUrl(subUrl)
  }

  return (
    <SubscriptionsSection
      subscriptions={subscriptions}
      loading={loading}
      saving={saving}
      editingId={editingId}
      name={name}
      url={url}
      onChangeName={setName}
      onChangeUrl={setUrl}
      onResetForm={resetForm}
      onSubmit={handleSubmit}
      onReload={() => {
        void load()
      }}
      onEdit={(sub) => {
        handleEdit(sub.id, sub.name, sub.url)
      }}
      onDelete={(id) => {
        if (!window.confirm('确认删除该订阅？')) return
        void remove(id)
      }}
      onActivate={(id) => {
        void activate(id)
      }}
      onFetch={(id) => {
        void fetchRemote(id)
      }}
    />
  )
}

export default SubscriptionsPage

