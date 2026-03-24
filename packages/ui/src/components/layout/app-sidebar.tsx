"use client"

import { useState, useRef, useEffect } from "react"
import Link from "next/link"
import { usePathname } from "next/navigation"
import {
  LogOut,
  ChevronsUpDown,
  type LucideIcon,
} from "lucide-react"
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
} from "../../components/ui/sidebar"

export interface NavItem {
  title: string
  href: string
  icon: LucideIcon
  adminOnly?: boolean
}

export interface AppSidebarUser {
  username: string
  email?: string
  role?: string
}

export interface AppSidebarProps {
  /** Navigation items to render in the sidebar */
  navItems: NavItem[]
  /** Current authenticated user (null hides the footer) */
  user: AppSidebarUser | null
  /** Called when user clicks sign out */
  onLogout: () => void
  /** Brand icon component rendered in the header */
  brandIcon: React.ReactNode
  /** Brand name displayed next to the icon */
  brandName: string
  /** Group label above the nav items */
  groupLabel?: string
}

export function AppSidebar({
  navItems,
  user,
  onLogout,
  brandIcon,
  brandName,
  groupLabel = "Navigation",
}: AppSidebarProps) {
  const pathname = usePathname()
  const [menuOpen, setMenuOpen] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)

  // UI-only filter -- the backend MUST enforce authorization on admin endpoints independently.
  const filteredNav = navItems.filter((item) => {
    if (item.adminOnly) {
      return user?.role === "admin"
    }
    return true
  })

  const initials = user?.username
    ? user.username.slice(0, 2).toUpperCase()
    : "?"

  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false)
      }
    }
    if (menuOpen) {
      document.addEventListener("mousedown", handleClickOutside)
      return () => document.removeEventListener("mousedown", handleClickOutside)
    }
  }, [menuOpen])

  return (
    <Sidebar>
      <SidebarHeader className="border-b border-sidebar-border px-4 h-14 flex items-center">
        <Link href="/" className="flex items-center gap-2">
          {brandIcon}
          <span className="text-lg font-bold tracking-tight">{brandName}</span>
        </Link>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel>{groupLabel}</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {filteredNav.map((item) => {
                const isActive =
                  item.href === "/"
                    ? pathname === "/"
                    : pathname.startsWith(item.href)
                return (
                  <SidebarMenuItem key={item.href}>
                    <SidebarMenuButton isActive={isActive} tooltip={item.title} render={<Link href={item.href} />}>
                        <item.icon className="h-4 w-4" />
                        <span>{item.title}</span>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                )
              })}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
      {user && (
        <SidebarFooter className="border-t border-sidebar-border">
          <div className="relative" ref={menuRef}>
            {menuOpen && (
              <div className="absolute bottom-full left-0 mb-2 w-full min-w-[180px] rounded-lg border border-border bg-popover p-1 shadow-lg">
                <div className="px-2 py-1.5">
                  <p className="text-sm font-medium">{user.username}</p>
                  <p className="text-xs text-muted-foreground capitalize">{user.role}</p>
                </div>
                <div className="my-1 h-px bg-border" />
                <button
                  type="button"
                  onClick={() => {
                    setMenuOpen(false)
                    onLogout()
                  }}
                  className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-accent hover:text-accent-foreground"
                >
                  <LogOut className="h-4 w-4" />
                  Sign out
                </button>
              </div>
            )}
            <button
              type="button"
              onClick={() => setMenuOpen(!menuOpen)}
              className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
            >
              <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-medium text-muted-foreground">
                {initials}
              </div>
              <div className="flex flex-col flex-1 text-left text-sm leading-tight">
                <span className="truncate font-medium">{user.username}</span>
                {user.email && (
                  <span className="truncate text-xs text-muted-foreground">{user.email}</span>
                )}
              </div>
              <ChevronsUpDown className="ml-auto h-4 w-4 text-muted-foreground" />
            </button>
          </div>
        </SidebarFooter>
      )}
      <SidebarRail />
    </Sidebar>
  )
}
