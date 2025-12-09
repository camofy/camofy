import { useNotifications } from '../context/NotificationContext'

function NotificationBar() {
  const { error, success, clearError, clearSuccess } = useNotifications()

  if (!error && !success) {
    return null
  }

  return (
    <div className="mb-3 space-y-1 text-xs">
      {error && (
        <div className="flex items-start justify-between rounded-md border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-rose-100">
          <p className="mr-2 break-words">{error}</p>
          <button
            type="button"
            className="ml-auto text-rose-200/80 hover:text-rose-50"
            onClick={clearError}
          >
            关闭
          </button>
        </div>
      )}
      {success && (
        <div className="flex items-start justify-between rounded-md border border-emerald-500/40 bg-emerald-500/10 px-3 py-2 text-emerald-100">
          <p className="mr-2 break-words">{success}</p>
          <button
            type="button"
            className="ml-auto text-emerald-200/80 hover:text-emerald-50"
            onClick={clearSuccess}
          >
            关闭
          </button>
        </div>
      )}
    </div>
  )
}

export default NotificationBar

