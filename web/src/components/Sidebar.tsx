import { NavLink } from 'react-router-dom'
import { NAV_ITEMS } from '../nav'
import { CollapseIcon, NAV_ICONS } from './NavIcons'

type SidebarProps = {
  collapsed: boolean
  mobileOpen: boolean
  onToggleCollapsed: () => void
  onCloseMobile: () => void
}

function Sidebar({
  collapsed,
  mobileOpen,
  onToggleCollapsed,
  onCloseMobile,
}: SidebarProps) {
  return (
    <>
      <div
        className={[
          'fixed inset-0 z-30 bg-black/30 transition-opacity lg:hidden',
          mobileOpen ? 'opacity-100' : 'pointer-events-none opacity-0',
        ].join(' ')}
        onClick={onCloseMobile}
      />

      <aside
        className={[
          'fixed inset-y-0 left-0 z-40 flex h-full flex-col bg-[color:var(--color-sidebar)] transition-all duration-200 lg:static lg:translate-x-0',
          mobileOpen ? 'translate-x-0' : '-translate-x-full',
          collapsed ? 'w-[88px]' : 'w-[248px]',
        ].join(' ')}
      >
        <div
          className={[
            'flex h-20 items-center',
            collapsed ? 'justify-center px-2' : 'gap-3 px-6',
          ].join(' ')}
        >
          <span className="brand-mark h-11 w-11 rounded-2xl">C</span>
          {!collapsed && (
            <div className="text-lg min-w-0 font-semibold">Camofy</div>
          )}
        </div>

        <nav className="flex-1 space-y-2 overflow-y-auto px-3 py-4">
          {NAV_ITEMS.map((item) => {
            const Icon = NAV_ICONS[item.to]
            return (
              <NavLink
                key={item.to}
                to={item.to}
                title={collapsed ? item.label : undefined}
                onClick={onCloseMobile}
                className={({ isActive }) =>
                  [
                    'text-lg flex items-center rounded-2xl px-4 py-3 transition-colors',
                    collapsed ? 'justify-center' : 'gap-3',
                    isActive
                      ? 'bg-white text-[color:var(--color-text-main)] shadow-[0_8px_24px_rgba(23,23,23,0.06)]'
                      : 'text-[color:var(--color-text-muted)] hover:bg-white/70 hover:text-[color:var(--color-text-main)]',
                  ].join(' ')
                }
              >
                <Icon className="h-5 w-5 shrink-0" />
                {!collapsed && <span className="truncate">{item.label}</span>}
              </NavLink>
            )
          })}
        </nav>

        <div className="hidden p-3 lg:block">
          <button
            type="button"
            onClick={onToggleCollapsed}
            className="flex w-full items-center justify-center rounded-2xl px-3 py-3 text-[color:var(--color-text-soft)] hover:bg-white hover:text-[color:var(--color-text-main)]"
            aria-label={collapsed ? '展开侧栏' : '收起侧栏'}
          >
            <CollapseIcon className={collapsed ? 'h-5 w-5 rotate-180' : 'h-5 w-5'} />
          </button>
        </div>
      </aside>
    </>
  )
}

export default Sidebar
