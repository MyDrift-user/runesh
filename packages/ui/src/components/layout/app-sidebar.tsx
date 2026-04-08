"use client"

import Link from "next/link"
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
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarRail,
} from "@/components/ui/sidebar"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Avatar, AvatarFallback } from "@/components/ui/avatar"

// ── Types ────────────────────────────────────────────────────────────────────

export interface NavItem {
  title: string
  href: string
  icon: LucideIcon
  /** When true the item is only shown to users with `role === "admin"`.
   * UI-only filter. The backend MUST enforce authorization independently. */
  adminOnly?: boolean
  /** Optional group label. Items sharing a label are rendered together
   * under that group heading; items without a label go in the default group. */
  group?: string
}

export interface AppSidebarUser {
  username: string
  email?: string
  role?: string
}

export interface AppSidebarProps {
  /** Navigation items rendered in the sidebar body. */
  navItems: NavItem[]
  /** Current authenticated user. When `null`, the profile footer is hidden. */
  user: AppSidebarUser | null
  /** Brand icon component rendered in the sidebar header. */
  brandIcon: React.ReactNode
  /** Brand name displayed next to the icon. */
  brandName: string
  /** Default group label, applied to nav items that don't set their own. */
  defaultGroupLabel?: string

  // ── Profile footer actions ────────────────────────────────────────────────
  /** Called when the user clicks "Sign out". */
  onLogout?: () => void
  /** Optional href for the "Account" entry. Hidden when omitted. */
  accountHref?: string
  /** Optional href for the "Settings" entry. Hidden when omitted. */
  settingsHref?: string
  /** Show the dark/light theme toggle in the profile dropdown. Default `true`. */
  enableThemeToggle?: boolean
  /** Extra dropdown items rendered above the Sign out action. */
  profileExtra?: React.ReactNode
}

// ── Component ────────────────────────────────────────────────────────────────

/**
 * Standard RUNESH dashboard sidebar.
 *
 * Renders a brand header, grouped navigation, and an optional profile
 * footer with a shadcn-based dropdown menu (account, settings, theme
 * toggle, sign out). Pair with [`DashboardShell`] for the full layout.
 */
export function AppSidebar({
  navItems,
  user,
  brandIcon,
  brandName,
  defaultGroupLabel = "Navigation",
  onLogout,
  accountHref,
  settingsHref,
  enableThemeToggle = true,
  profileExtra,
}: AppSidebarProps) {
  const pathname = usePathname()

  // UI-only role filter. Backend authorization is the source of truth.
  const visibleNav = navItems.filter((item) => {
    if (item.adminOnly && user?.role !== "admin") return false
    return true
  })

  // Group nav items by their `group` (or fall back to `defaultGroupLabel`),
  // preserving the original order within each group.
  const groups = new Map<string, NavItem[]>()
  for (const item of visibleNav) {
    const label = item.group ?? defaultGroupLabel
    const bucket = groups.get(label) ?? []
    bucket.push(item)
    groups.set(label, bucket)
  }

  return (
    <Sidebar>
      <SidebarHeader className="border-b border-sidebar-border px-4 h-14 flex items-center">
        <Link href="/" className="flex items-center gap-2">
          {brandIcon}
          <span className="text-lg font-bold tracking-tight">{brandName}</span>
        </Link>
      </SidebarHeader>

      <SidebarContent>
        {Array.from(groups.entries()).map(([label, items]) => (
          <SidebarGroup key={label}>
            <SidebarGroupLabel>{label}</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {items.map((item) => {
                  const isActive =
                    item.href === "/" ? pathname === "/" : pathname.startsWith(item.href)
                  return (
                    <SidebarMenuItem key={item.href}>
                      <SidebarMenuButton asChild isActive={isActive} tooltip={item.title}>
                        <Link href={item.href}>
                          <item.icon className="h-4 w-4" />
                          <span>{item.title}</span>
                        </Link>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  )
                })}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        ))}
      </SidebarContent>

      {user && (
        <SidebarFooter className="border-t border-sidebar-border p-2">
          <ProfileMenu
            user={user}
            onLogout={onLogout}
            accountHref={accountHref}
            settingsHref={settingsHref}
            enableThemeToggle={enableThemeToggle}
            extra={profileExtra}
          />
        </SidebarFooter>
      )}

      <SidebarRail />
    </Sidebar>
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
}

function ProfileMenu({
  user,
  onLogout,
  accountHref,
  settingsHref,
  enableThemeToggle,
  extra,
}: ProfileMenuProps) {
  const initials = user.username.slice(0, 2).toUpperCase()

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-2 focus-visible:ring-ring"
        >
          <Avatar className="h-7 w-7">
            <AvatarFallback className="text-xs font-medium">{initials}</AvatarFallback>
          </Avatar>
          <div className="flex flex-1 flex-col text-left text-sm leading-tight overflow-hidden">
            <span className="truncate font-medium">{user.username}</span>
            {user.email && (
              <span className="truncate text-xs text-muted-foreground">{user.email}</span>
            )}
          </div>
          <ChevronsUpDown className="ml-auto h-4 w-4 shrink-0 text-muted-foreground" />
        </button>
      </DropdownMenuTrigger>

      <DropdownMenuContent
        className="w-(--radix-dropdown-menu-trigger-width) min-w-56"
        side="right"
        align="end"
        sideOffset={8}
      >
        <DropdownMenuLabel className="font-normal">
          <div className="flex flex-col gap-0.5">
            <span className="text-sm font-medium">{user.username}</span>
            {user.email && (
              <span className="text-xs text-muted-foreground">{user.email}</span>
            )}
            {user.role && (
              <span className="text-xs text-muted-foreground capitalize">{user.role}</span>
            )}
          </div>
        </DropdownMenuLabel>

        {(accountHref || settingsHref) && <DropdownMenuSeparator />}

        <DropdownMenuGroup>
          {accountHref && (
            <DropdownMenuItem asChild>
              <Link href={accountHref}>
                <UserIcon className="h-4 w-4" />
                Account
              </Link>
            </DropdownMenuItem>
          )}
          {settingsHref && (
            <DropdownMenuItem asChild>
              <Link href={settingsHref}>
                <Settings className="h-4 w-4" />
                Settings
              </Link>
            </DropdownMenuItem>
          )}
        </DropdownMenuGroup>

        {enableThemeToggle && (
          <>
            <DropdownMenuSeparator />
            <ThemeToggleItem />
          </>
        )}

        {extra && (
          <>
            <DropdownMenuSeparator />
            {extra}
          </>
        )}

        {onLogout && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onClick={onLogout}>
              <LogOut className="h-4 w-4" />
              Sign out
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function ThemeToggleItem() {
  const { resolvedTheme, setTheme } = useTheme()
  const isDark = resolvedTheme === "dark"
  return (
    <DropdownMenuItem
      onSelect={(e) => {
        // Don't close the menu so the user sees the icon flip.
        e.preventDefault()
        setTheme(isDark ? "light" : "dark")
      }}
    >
      {isDark ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
      {isDark ? "Light mode" : "Dark mode"}
    </DropdownMenuItem>
  )
}
