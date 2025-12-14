import { useEffect } from 'react'
import CoreSection from '../components/CoreSection'
import { useCore } from '../hooks/useCore'

function CorePage() {
  const {
    coreInfo,
    coreStatus,
    loading,
    actionLoading,
    operationState,
    refresh,
    download,
    start,
    stop,
    restart,
  } = useCore()

  useEffect(() => {
    void refresh()
  }, [refresh])

  return (
    <CoreSection
      coreInfo={coreInfo}
      coreStatus={coreStatus}
      coreLoading={loading}
      coreActionLoading={actionLoading}
      coreOperation={operationState}
      onRefresh={() => {
        void refresh()
      }}
      onDownload={() => {
        void download()
      }}
      onStart={() => {
        void start()
      }}
      onStop={() => {
        void stop()
      }}
      onRestart={() => {
        void restart()
      }}
    />
  )
}

export default CorePage
