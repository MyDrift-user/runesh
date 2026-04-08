"use client"

/**
 * Self-contained dashboard sidebar.
 *
 * Intentionally has NO dependency on the consumer's shadcn primitives.
 * Works in any Next.js project regardless of which shadcn flavor (Radix
 * `asChild` vs base-ui `render`) the consumer's `@/components/ui/*` are
 * built on. Only requirements:
 *   - Tailwind CSS in the consumer
 *   - lucide-react and next-themes as peer deps
 *   - A router link component passed as `linkComponent`
 *
 * Pair with [`DashboardShell`] which is also self-contained.
 */

import * as React from "react"
import { usePathname } from "next/navigation"
import {
  ChevronsUpDown,
  LogOut,
  Moon,
  Settings,
  Sun,
  User as UserIcon,
  type LucideIcon,
} from "lucide-react"
import { useTheme } from "next-themes"
import {
  FloatingFocusManager,
  FloatingPortal,
  autoUpdate,
  offset,
  shift,
  useClick,
  useDismiss,
  useFloating,
  useInteractions,
  useRole,
} from "@floating-ui/react"

// ── Types ────────────────────────────────────────────────────────────────────

export interface NavItem {
  title: string
  href: string
  icon: LucideIcon
  /** UI-only filter. Backend MUST enforce authorization independently. */
  adminOnly?: boolean
  /** Optional group label, items sharing a label render under one heading. */
  group?: string
}

export interface AppSidebarUser {
  username: string
  email?: string
  role?: string
}

/**
 * Generic link component contract. Pass `Link` from `next/link` or any
 * router. Defaults to a plain `<a>` for SSR safety.
 */
export type LinkLikeProps = React.AnchorHTMLAttributes<HTMLAnchorElement> & {
  href: string
  children?: React.ReactNode
}
export type LinkLike = React.ComponentType<LinkLikeProps>

const DefaultLink: LinkLike = ({ href, children, ...rest }) => (
  <a href={href} {...rest}>
    {children}
  </a>
)

export interface AppSidebarProps {
  navItems: NavItem[]
  user: AppSidebarUser | null
  brandIcon: React.ReactNode
  brandName: string
  /** Default group label, applied to nav items that don't set their own. */
  defaultGroupLabel?: string
  /** Router link component (e.g. `next/link`'s `Link`). Defaults to `<a>`. */
  linkComponent?: LinkLike
  /** Called when "Sign out" is clicked. Hidden when omitted. */
  onLogout?: () => void
  /** Optional href for the "Account" entry. */
  accountHref?: string
  /** Optional href for the "Settings" entry. */
  settingsHref?: string
  /** Show the dark/light theme toggle in the profile dropdown. Default `true`. */
  enableThemeToggle?: boolean
  /** Extra dropdown items rendered above Sign out. */
  profileExtra?: React.ReactNode
}

// ── Sidebar component ───────────────────────────────────────────────────────

export function AppSidebar({
  navItems,
  user,
  brandIcon,
  brandName,
  defaultGroupLabel = "Navigation",
  linkComponent: Link = DefaultLink,
  onLogout,
  accountHref,
  settingsHref,
  enableThemeToggle = true,
  profileExtra,
}: AppSidebarProps) {
  const pathname = usePathname()

  const visibleNav = navItems.filter(
    (item) => !item.adminOnly || user?.role === "admin"
  )

  // Group items by `group` (or default), preserve original order.
  const groups = new Map<string, NavItem[]>()
  for (const item of visibleNav) {
    const label = item.group ?? defaultGroupLabel
    const bucket = groups.get(label) ?? []
    bucket.push(item)
    groups.set(label, bucket)
  }

  // When every nav item falls into the single default group, the label is
  // visual noise (a heavy "NAVIGATION" header above one list). Suppress it.
  // Only render group labels when at least one item explicitly opts in via
  // its own `group` field, which signals multi-section intent.
  const showGroupLabels = navItems.some((item) => item.group != null)

  return (
    <aside
      data-slot="runesh-app-sidebar"
      className="flex h-screen w-64 shrink-0 flex-col border-r border-border bg-sidebar text-sidebar-foreground"
    >
      {/* Header */}
      <div className="flex h-14 shrink-0 items-center gap-2 border-b border-border px-4">
        <Link href="/" className="flex items-center gap-2">
          {brandIcon}
          <span className="text-lg font-bold tracking-tight">{brandName}</span>
        </Link>
      </div>

      {/* Nav */}
      <nav className="flex-1 overflow-y-auto p-2">
        {Array.from(groups.entries()).map(([label, items]) => (
          <div key={label} className="mb-2">
            {showGroupLabels && (
              <div className="px-2 pb-1 pt-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground/70">
                {label}
              </div>
            )}
            <ul className="flex flex-col gap-0.5">
              {items.map((item) => {
                const isActive =
                  item.href === "/"
                    ? pathname === "/"
                    : pathname.startsWith(item.href)
                return (
                  <li key={item.href}>
                    <Link
                      href={item.href}
                      data-active={isActive ? "true" : undefined}
                      className={[
                        "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm",
                        "outline-none transition-colors",
                        "hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                        "focus-visible:ring-2 focus-visible:ring-ring",
                        "data-[active=true]:bg-sidebar-accent data-[active=true]:font-medium data-[active=true]:text-sidebar-accent-foreground",
                      ].join(" ")}
                    >
                      <item.icon className="h-4 w-4 shrink-0" />
                      <span className="truncate">{item.title}</span>
                    </Link>
                  </li>
                )
              })}
            </ul>
          </div>
        ))}
      </nav>

      {/* Profile footer */}
      {user && (
        <div className="border-t border-border p-2">
          <ProfileMenu
            user={user}
            onLogout={onLogout}
            accountHref={accountHref}
            settingsHref={settingsHref}
            enableThemeToggle={enableThemeToggle}
            extra={profileExtra}
            LinkComponent={Link}
          />
        </div>
      )}
    </aside>
  )
}

// ── Profile dropdown ─────────────────────────────────────────────────────────

interface ProfileMenuProps {
  user: AppSidebarUser
  onLogout?: () => void
  accountHref?: string
  settingsHref?: string
  enableThemeToggle: boolean
  extra?: React.ReactNode
  LinkComponent: LinkLike
}

function ProfileMenu({
  user,
  onLogout,
  accountHref,
  settingsHref,
  enableThemeToggle,
  extra,
  LinkComponent,
}: ProfileMenuProps) {
  const [open, setOpen] = React.useState(false)

  // floating-ui handles the heavy lifting:
  //   - position the menu relative to the trigger
  //   - autoUpdate keeps it positioned on scroll/resize
  //   - shift() pushes it back into the viewport on collisions
  //   - offset(8) gives a small gap above the trigger
  //   - FloatingPortal renders into document.body so the sidebar's
  //     overflow constraints can never clip it
  const { refs, floatingStyles, context } = useFloating({
    open,
    onOpenChange: setOpen,
    placement: "top-start",
    middleware: [offset(8), shift({ padding: 8 })],
    whileElementsMounted: autoUpdate,
  })

  const click = useClick(context)
  const dismiss = useDismiss(context)
  const role = useRole(context, { role: "menu" })

  const { getReferenceProps, getFloatingProps } = useInteractions([
    click,
    dismiss,
    role,
  ])

  const initials = user.username.slice(0, 2).toUpperCase()

  return (
    <>
      <button
        ref={refs.setReference}
        type="button"
        {...getReferenceProps()}
        className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-2 focus-visible:ring-ring"
      >
        <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-medium text-muted-foreground">
          {initials}
        </div>
        <div className="flex flex-1 flex-col overflow-hidden text-left text-sm leading-tight">
          <span className="truncate font-medium">{user.username}</span>
          {user.email && (
            <span className="truncate text-xs text-muted-foreground">
              {user.email}
            </span>
          )}
        </div>
        <ChevronsUpDown className="ml-auto h-4 w-4 shrink-0 text-muted-foreground" />
      </button>

      {open && (
        <FloatingPortal>
          <FloatingFocusManager context={context} modal={false}>
            <div
              ref={refs.setFloating}
              style={floatingStyles}
              {...getFloatingProps()}
              className="z-50 min-w-56 overflow-hidden rounded-md border border-border bg-popover text-popover-foreground shadow-lg outline-none"
            >
              <div className="px-3 py-2">
                <div className="text-sm font-medium">{user.username}</div>
                {user.email && (
                  <div className="text-xs text-muted-foreground">
                    {user.email}
                  </div>
                )}
                {user.role && (
                  <div className="text-xs capitalize text-muted-foreground">
                    {user.role}
                  </div>
                )}
              </div>

              {(accountHref || settingsHref) && <MenuSeparator />}

              {accountHref && (
                <LinkComponent
                  href={accountHref}
                  className="flex items-center gap-2 px-3 py-2 text-sm hover:bg-accent hover:text-accent-foreground"
                  onClick={() => setOpen(false)}
                >
                  <UserIcon className="h-4 w-4" />
                  Account
                </LinkComponent>
              )}
              {settingsHref && (
                <LinkComponent
                  href={settingsHref}
                  className="flex items-center gap-2 px-3 py-2 text-sm hover:bg-accent hover:text-accent-foreground"
                  onClick={() => setOpen(false)}
                >
                  <Settings className="h-4 w-4" />
                  Settings
                </LinkComponent>
              )}

              {enableThemeToggle && <ThemeToggleItem />}

              {extra && (
                <>
                  <MenuSeparator />
                  {extra}
                </>
              )}

              {onLogout && (
                <>
                  <MenuSeparator />
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      setOpen(false)
                      onLogout()
                    }}
                    className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-accent hover:text-accent-foreground"
                  >
                    <LogOut className="h-4 w-4" />
                    Sign out
                  </button>
                </>
              )}
            </div>
          </FloatingFocusManager>
        </FloatingPortal>
      )}
    </>
  )
}

function MenuSeparator() {
  return <div className="my-1 h-px bg-border" />
}

function ThemeToggleItem() {
  const { resolvedTheme, setTheme } = useTheme()
  const isDark = resolvedTheme === "dark"
  return (
    <button
      type="button"
      role="menuitem"
      onClick={() => setTheme(isDark ? "light" : "dark")}
      className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-accent hover:text-accent-foreground"
    >
      {isDark ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
      {isDark ? "Light mode" : "Dark mode"}
    </button>
  )
}
