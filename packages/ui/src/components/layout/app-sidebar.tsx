"use client"

/**
 * Self-contained dashboard sidebar.
 *
 * Sized and spaced to match the canonical shadcn `Sidebar` primitive
 * (16rem width, h-8 menu items, h-12 profile/team-switcher row, p-2
 * section padding, [&>svg]:size-4 icons), but ships with NO dependency
 * on the consumer's shadcn primitives so it works regardless of which
 * shadcn flavor (Radix `asChild` or base-ui `render`) the consumer ships.
 *
 * Pair with [`DashboardShell`].
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

// ── Style tokens ─────────────────────────────────────────────────────────────
//
// Centralised so the whole component shares one rhythm. These match the
// canonical shadcn Sidebar primitive defaults.

const SIDEBAR_WIDTH = "w-64" // 16rem / 256px
const HEADER_HEIGHT = "h-14" // 56px, matches DashboardShell toolbar
const SECTION_PAD = "p-2" // 8px outer padding for the nav body
const MENU_ITEM_BASE = [
  // Layout
  "group/menu-item flex h-8 w-full items-center gap-2 overflow-hidden",
  "rounded-md px-2 text-left text-sm",
  // Behaviour
  "outline-none transition-colors",
  "focus-visible:ring-2 focus-visible:ring-ring",
  // Default state. Uses generic `accent` tokens (always defined by shadcn
  // base theme) instead of `sidebar-accent` which not every consumer wires
  // into Tailwind v4's --color-* namespace.
  "text-foreground/80",
  "hover:bg-accent hover:text-accent-foreground",
  // Active state
  "data-[active=true]:bg-accent data-[active=true]:font-medium data-[active=true]:text-accent-foreground",
  // Icon: fixed 16x16 slot, never shifts even if title length varies
  "[&>svg]:size-4 [&>svg]:shrink-0",
  // Truncate the label
  "[&>span]:truncate",
].join(" ")

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

  return (
    <aside
      data-slot="runesh-app-sidebar"
      className={`flex h-screen ${SIDEBAR_WIDTH} shrink-0 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground`}
    >
      {/* Header */}
      <div
        className={`flex ${HEADER_HEIGHT} shrink-0 items-center gap-2 border-b border-sidebar-border px-4`}
      >
        <Link href="/" className="flex items-center gap-2 outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-md">
          <div className="flex shrink-0 items-center justify-center">
            {brandIcon}
          </div>
          <span className="truncate text-base font-semibold tracking-tight">
            {brandName}
          </span>
        </Link>
      </div>

      {/* Nav */}
      <nav className={`flex flex-1 flex-col gap-1 overflow-y-auto ${SECTION_PAD}`}>
        {Array.from(groups.entries()).map(([label, items], i) => (
          <div key={label} className={i === 0 ? "" : "mt-3"}>
            {/* Section label. Always rendered so consumers can divide nav
                into permission tiers (User / Moderator / Admin). Sized
                small and quiet so it never dominates the items below. */}
            <div className="px-2 pt-2 pb-1 text-[10px] font-medium uppercase tracking-wider text-muted-foreground/60">
              {label}
            </div>
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
                      className={MENU_ITEM_BASE}
                    >
                      <item.icon />
                      <span>{item.title}</span>
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
        <div className={`border-t border-sidebar-border ${SECTION_PAD}`}>
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

const PROFILE_BUTTON = [
  // Layout: same h-12 as shadcn's `lg` SidebarMenuButton size
  "flex h-12 w-full items-center gap-2 overflow-hidden",
  "rounded-md p-2 text-left text-sm",
  // Behaviour
  "outline-none transition-colors",
  "focus-visible:ring-2 focus-visible:ring-ring",
  // States. Generic accent tokens for cross-consumer compatibility.
  "text-foreground",
  "hover:bg-accent hover:text-accent-foreground",
  "data-[state=open]:bg-accent data-[state=open]:text-accent-foreground",
].join(" ")

const MENU_LINK = [
  "flex h-9 w-full items-center gap-2 px-3 text-sm",
  "outline-none transition-colors",
  "text-popover-foreground",
  "hover:bg-accent hover:text-accent-foreground",
  "focus-visible:bg-accent focus-visible:text-accent-foreground",
  "[&>svg]:size-4 [&>svg]:shrink-0",
].join(" ")

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

  // floating-ui handles positioning, autoUpdate, collision detection,
  // portaling out of the sidebar overflow, and a11y interactions.
  // top-start opens the menu directly above the trigger button — the
  // standard pattern Vercel / Linear / Cal use for sidebar profile menus.
  // shift() pushes the menu back into the viewport on collision instead
  // of clipping, and FloatingPortal renders to body so the sidebar's
  // overflow can never clip it.
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
        data-state={open ? "open" : "closed"}
        {...getReferenceProps()}
        className={PROFILE_BUTTON}
      >
        {/* Avatar */}
        <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-accent text-xs font-medium text-accent-foreground">
          {initials}
        </div>
        {/* Text column */}
        <div className="grid flex-1 text-left text-sm leading-tight">
          <span className="truncate font-medium">{user.username}</span>
          {user.email && (
            <span className="truncate text-xs text-muted-foreground">
              {user.email}
            </span>
          )}
        </div>
        <ChevronsUpDown className="ml-auto size-4 shrink-0 opacity-60" />
      </button>

      {open && (
        <FloatingPortal>
          <FloatingFocusManager context={context} modal={false}>
            <div
              ref={refs.setFloating}
              style={floatingStyles}
              {...getFloatingProps()}
              className="z-50 min-w-60 overflow-hidden rounded-lg border border-border bg-popover py-1 text-popover-foreground shadow-lg outline-none"
            >
              {/* Header */}
              <div className="flex items-center gap-2 px-3 py-2">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-accent text-xs font-medium text-accent-foreground">
                  {initials}
                </div>
                <div className="grid flex-1 text-left text-sm leading-tight">
                  <span className="truncate font-medium">{user.username}</span>
                  {user.email && (
                    <span className="truncate text-xs text-muted-foreground">
                      {user.email}
                    </span>
                  )}
                </div>
              </div>

              {(accountHref || settingsHref || enableThemeToggle || extra) && (
                <MenuSeparator />
              )}

              {accountHref && (
                <LinkComponent
                  href={accountHref}
                  className={MENU_LINK}
                  onClick={() => setOpen(false)}
                >
                  <UserIcon />
                  <span>Account</span>
                </LinkComponent>
              )}
              {settingsHref && (
                <LinkComponent
                  href={settingsHref}
                  className={MENU_LINK}
                  onClick={() => setOpen(false)}
                >
                  <Settings />
                  <span>Settings</span>
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
                    className={MENU_LINK}
                  >
                    <LogOut />
                    <span>Sign out</span>
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
      className={MENU_LINK}
    >
      {isDark ? <Sun /> : <Moon />}
      <span>{isDark ? "Light mode" : "Dark mode"}</span>
    </button>
  )
}
