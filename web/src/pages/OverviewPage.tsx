import { useEffect } from 'react'
import { useAuth } from '../context/AuthContext'
import { useSubscriptions } from '../hooks/useSubscriptions'
import { useCore } from '../hooks/useCore'
import { useScheduler } from '../hooks/useScheduler'
import SystemStatusSection from '../components/SystemStatusSection'
import SchedulerSection from '../components/SchedulerSection'

function OverviewPage() {
  const { passwordSet } = useAuth()
  const { subscriptions, load: loadSubscriptions } = useSubscriptions()
  const {
    coreStatus,
    loading: coreLoading,
    refresh: refreshCore,
  } = useCore()
  const {
    subscriptionTask,
    geoipTask,
    saving,
    setSubscriptionTask,
    setGeoipTask,
    load: loadScheduler,
    save: saveScheduler,
  } = useScheduler()

  useEffect(() => {
    void loadSubscriptions()
    void refreshCore()
    void loadScheduler()
  }, [loadSubscriptions, refreshCore, loadScheduler])

  return (
    <>
      <SystemStatusSection
        coreStatus={coreStatus}
        subscriptionsCount={subscriptions.length}
        passwordSet={passwordSet}
      />
      <SchedulerSection
        subscriptionTask={subscriptionTask}
        geoipTask={geoipTask}
        onChangeSubscriptionTask={setSubscriptionTask}
        onChangeGeoipTask={setGeoipTask}
        onSave={saveScheduler}
        saving={saving}
      />
      {coreLoading && (
        <p className="text-xs text-[color:var(--color-text-soft)]">
          正在刷新内核状态…
        </p>
      )}
    </>
  )
}

export default OverviewPage
