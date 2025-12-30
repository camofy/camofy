import { useNotifications } from '../context/NotificationContext'

function NotificationBar() {
  const { error, success, clearError, clearSuccess } = useNotifications()

  if (!error && !success) {
    return null
  }

  return (
    <div className="mb-3 space-y-1 text-xs">
      {error && (
        <div className="flex items-start justify-between rounded-md border border-[color:var(--color-danger)] bg-[color:var(--color-danger-soft)] px-3 py-2 text-[color:var(--color-danger)]">
          <p className="mr-2 break-words">{error}</p>
          <button
            type="button"
            className="ml-auto text-[color:var(--color-danger)] hover:text-[#a85f51]"
            onClick={clearError}
          >
            关闭
          </button>
        </div>
      )}
      {success && (
        <div className="flex items-start justify-between rounded-md border border-[color:var(--color-success)] bg-[color:var(--color-success-soft)] px-3 py-2 text-[color:var(--color-success)]">
          <p className="mr-2 break-words">{success}</p>
          <button
            type="button"
            className="ml-auto text-[color:var(--color-success)] hover:text-[#5f7757]"
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
