import { findNavItem } from '../nav'
import { MenuIcon } from './NavIcons'

type TopbarProps = {
  pathname: string
  passwordSet: boolean
  authToken: string | null
  onLogout: () => void
  onOpenMobile: () => void
}

function Topbar({
  pathname,
  passwordSet,
  authToken,
  onLogout,
  onOpenMobile,
}: TopbarProps) {
  const current = findNavItem(pathname)

  return (
    <header className="flex h-20 shrink-0 items-center justify-between px-6">
      <div className="flex min-w-0 items-center gap-3">
        <button
          type="button"
          className="rounded-2xl p-2 text-[color:var(--color-text-main)] hover:bg-white lg:hidden"
          onClick={onOpenMobile}
          aria-label="打开菜单"
        >
          <MenuIcon className="h-6 w-6" />
        </button>
        <h1 className="truncate text-2xl font-semibold">
          {current?.label ?? 'Camofy'}
        </h1>
      </div>

      <div className="flex items-center gap-4 text-lg text-[color:var(--color-text-muted)]">
        {passwordSet && authToken ? (
          <button type="button" className="ui-btn-default" onClick={onLogout}>
            退出
          </button>
        ) : (
          <span>未设置密码</span>
        )}
      </div>
    </header>
  )
}

export default Topbar
