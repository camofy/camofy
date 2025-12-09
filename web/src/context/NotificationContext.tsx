import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useState,
} from 'react'

type NotificationContextValue = {
  error: string | null
  success: string | null
  notifyError: (message: string) => void
  notifySuccess: (message: string) => void
  clearError: () => void
  clearSuccess: () => void
  clearAll: () => void
}

const NotificationContext = createContext<NotificationContextValue | undefined>(
  undefined,
)

export function NotificationProvider({ children }: { children: ReactNode }) {
  const [error, setError] = useState<string | null>(null)
  const [success, setSuccess] = useState<string | null>(null)

  const notifyError = useCallback((message: string) => {
    setError(message)
    setSuccess(null)
  }, [])

  const notifySuccess = useCallback((message: string) => {
    setSuccess(message)
    setError(null)
  }, [])

  const clearError = useCallback(() => setError(null), [])
  const clearSuccess = useCallback(() => setSuccess(null), [])
  const clearAll = useCallback(() => {
    setError(null)
    setSuccess(null)
  }, [])

  return (
    <NotificationContext.Provider
      value={{
        error,
        success,
        notifyError,
        notifySuccess,
        clearError,
        clearSuccess,
        clearAll,
      }}
    >
      {children}
    </NotificationContext.Provider>
  )
}

export function useNotifications(): NotificationContextValue {
  const ctx = useContext(NotificationContext)
  if (!ctx) {
    throw new Error('useNotifications must be used within NotificationProvider')
  }
  return ctx
}

