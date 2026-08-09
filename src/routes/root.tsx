import { Trans } from '@lingui/react/macro'
import { Link, Outlet } from '@tanstack/react-router'
import { HomeIcon, SettingsIcon } from '@/components/icons'

function NavLink({
  to,
  icon,
  children,
}: {
  to: string
  icon: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <Link
      to={to}
      className="flex items-center gap-2.5 rounded-md px-3 py-2 text-sm font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground [&.active]:bg-primary/15 [&.active]:text-foreground"
      activeOptions={{ exact: to === '/' }}
    >
      {icon}
      {children}
    </Link>
  )
}

export function RootLayout() {
  return (
    <div className="flex h-screen">
      <aside className="flex w-56 shrink-0 flex-col gap-1 border-r border-border bg-card/40 p-3">
        <div className="px-3 py-3">
          <div className="text-sm font-semibold">HD2 Repatcher</div>
          <div className="text-xs text-muted-foreground">
            <Trans>Unit and audio mod repatcher</Trans>
          </div>
        </div>
        <NavLink to="/" icon={<HomeIcon width={16} height={16} />}>
          <Trans>Home</Trans>
        </NavLink>
        <NavLink to="/settings" icon={<SettingsIcon width={16} height={16} />}>
          <Trans>Settings</Trans>
        </NavLink>
      </aside>

      <main className="flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl p-8">
          <Outlet />
        </div>
      </main>
    </div>
  )
}
