export type NavPath =
  | '/overview'
  | '/subscriptions'
  | '/profiles'
  | '/core'
  | '/proxies'
  | '/logs'

export type NavItem = {
  to: NavPath
  label: string
  description: string
}

export const NAV_ITEMS: NavItem[] = [
  { to: '/overview', label: '总览', description: '' },
  { to: '/subscriptions', label: '订阅', description: '' },
  { to: '/profiles', label: '配置', description: '' },
  { to: '/core', label: '内核', description: '' },
  { to: '/proxies', label: '节点', description: '' },
  { to: '/logs', label: '日志', description: '' },
]

export function findNavItem(pathname: string): NavItem | undefined {
  return NAV_ITEMS.find(
    (item) => pathname === item.to || pathname.startsWith(`${item.to}/`),
  )
}
