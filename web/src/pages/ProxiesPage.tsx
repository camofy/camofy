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
    testing,
    testingNodes,
    testGroup,
  } = useProxies()

  useEffect(() => {
    void load()
  }, [load])

  return (
    <ProxyGroupsSection
      proxies={view}
      loading={loading}
      selecting={selecting}
      testing={testing}
      testingNodes={testingNodes}
      onReload={() => {
        void load()
      }}
      onSelectNode={(groupName, nodeName) => {
        void select(groupName, nodeName)
      }}
      onTestGroup={(groupName) => {
        void testGroup(groupName)
      }}
    />
  )
}

export default ProxiesPage
