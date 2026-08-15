import { useNotifications } from '../context/NotificationContext'

function NotificationBar() {
  const { error, success, clearError, clearSuccess } = useNotifications()

  if (!error && !success) {
    return null
  }

  return (
    <div className="shrink-0 space-y-2 px-6 pb-2">
      {error && (
        <div className="text-lg flex items-start justify-between rounded-2xl bg-[color:var(--color-danger-soft)] px-5 py-3 text-[color:var(--color-danger)]">
          <p className="mr-3 break-words">{error}</p>
          <button type="button" className="text-lg shrink-0 font-medium" onClick={clearError}>
            关闭
          </button>
        </div>
      )}
      {success && (
        <div className="text-lg flex items-start justify-between rounded-2xl bg-[color:var(--color-success-soft)] px-5 py-3 text-[color:var(--color-success)]">
          <p className="mr-3 break-words">{success}</p>
          <button type="button" className="text-lg shrink-0 font-medium" onClick={clearSuccess}>
            关闭
          </button>
        </div>
      )}
    </div>
  )
}

export default NotificationBar
