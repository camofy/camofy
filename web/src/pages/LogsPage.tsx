import { useEffect } from 'react'
import LogsSection from '../components/LogsSection'
import { useLogs } from '../hooks/useLogs'

function LogsPage() {
  const {
    appLog,
    mihomoLog,
    loading,
    load,
  } = useLogs()

  useEffect(() => {
    void load()
  }, [load])

  return (
    <LogsSection
      appLog={appLog}
      mihomoLog={mihomoLog}
      loading={loading}
      onReload={() => {
        void load()
      }}
    />
  )
}

export default LogsPage
