import { useEffect } from 'react'
import ProxyGroupsSection from '../components/ProxyGroupsSection'
import { useProxies } from '../hooks/useProxies'

function ProxiesPage() {
  const {
    view,
    loading,
    selecting,
    load,
    select,
  } = useProxies()

  useEffect(() => {
    void load()
  }, [load])

  return (
    <ProxyGroupsSection
      proxies={view}
      loading={loading}
      selecting={selecting}
      onReload={() => {
        void load()
      }}
      onSelectNode={(groupName, nodeName) => {
        void select(groupName, nodeName)
      }}
    />
  )
}

export default ProxiesPage
