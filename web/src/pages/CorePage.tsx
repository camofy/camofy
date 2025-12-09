import { useEffect } from 'react'
import CoreSection from '../components/CoreSection'
import { useCore } from '../hooks/useCore'

function CorePage() {
  const {
    coreInfo,
    coreStatus,
    loading,
    actionLoading,
    refresh,
    download,
    start,
    stop,
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
    />
  )
}

export default CorePage

