"use client"

/**
 * Self-contained dashboard shell layout.
 *
 * Renders a fixed sidebar on the left and a scrollable content area on the
 * right with a sticky toolbar header. Has NO dependency on shadcn primitives
 * (sidebar/tooltip/etc.) so it works in any consumer regardless of which
 * shadcn flavor they ship.
 *
 * Pair with [`AppSidebar`] for the standard RUNESH dashboard layout.
 */

import * as React from "react"

export interface DashboardShellProps {
  children: React.ReactNode
  /** The sidebar component to render (e.g. `<AppSidebar ... />`). */
  sidebar: React.ReactNode
  /** Optional global element rendered after the main area, e.g. a search command palette. */
  searchBar?: React.ReactNode
  /** Keyboard shortcut hint displayed in the toolbar (e.g. `<kbd>Ctrl K</kbd>`). */
  shortcutHint?: React.ReactNode
  /** Additional content to render in the toolbar header. */
  toolbarExtra?: React.ReactNode
  /** Tailwind class for the main content area. Default `"p-4 md:p-6"`. */
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
    <div data-slot="runesh-dashboard-shell" className="flex h-screen w-full">
      {sidebar}
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <header className="sticky top-0 z-20 flex h-14 shrink-0 items-center gap-2 border-b border-border bg-background px-4">
          {toolbarExtra}
          <div className="flex-1" />
          {shortcutHint}
        </header>
        <main className={`min-w-0 flex-1 overflow-y-auto ${contentClassName}`}>
          {children}
        </main>
      </div>
      {searchBar}
    </div>
  )
}
