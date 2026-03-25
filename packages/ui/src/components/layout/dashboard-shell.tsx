"use client"

import { TooltipProvider } from "@/components/ui/tooltip"
import { SidebarProvider, SidebarInset, SidebarTrigger } from "@/components/ui/sidebar"

export interface DashboardShellProps {
  children: React.ReactNode
  /** The sidebar component to render (e.g. <AppSidebar />) */
  sidebar: React.ReactNode
  /** Optional search bar component (rendered globally, e.g. command palette) */
  searchBar?: React.ReactNode
  /** Keyboard shortcut hint shown in the toolbar (e.g. "Ctrl K") */
  shortcutHint?: React.ReactNode
  /** Additional content to render in the toolbar header */
  toolbarExtra?: React.ReactNode
  /** CSS class for the main content area */
  contentClassName?: string
}

export function DashboardShell({
  children,
  sidebar,
  searchBar,
  shortcutHint,
  toolbarExtra,
  contentClassName = "p-4 md:p-6",
}: DashboardShellProps) {
  return (
    <TooltipProvider>
      <SidebarProvider>
        {sidebar}
        <SidebarInset className="overflow-hidden">
          <header className="sticky top-0 z-20 flex h-14 shrink-0 items-center gap-2 border-b bg-background px-4">
            <SidebarTrigger className="-ml-1" />
            {toolbarExtra}
            <div className="flex-1" />
            {shortcutHint}
          </header>
          <main className={`flex-1 min-w-0 ${contentClassName}`}>
            {children}
          </main>
        </SidebarInset>
        {searchBar}
      </SidebarProvider>
    </TooltipProvider>
  )
}
