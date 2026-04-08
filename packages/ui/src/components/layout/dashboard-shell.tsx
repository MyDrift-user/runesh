"use client"

/**
 * Self-contained dashboard shell layout.
 *
 * Renders a sticky sidebar on the left and a scrollable content area on
 * the right with a sticky toolbar header. Backdrop-blurred header that
 * matches the canonical shadcn dashboard layout. No shadcn primitive
 * dependency, works in any consumer regardless of which shadcn flavor
 * they ship.
 *
 * Pair with [`AppSidebar`].
 */

import * as React from "react"
import { Search } from "lucide-react"

export interface DashboardShellProps {
  children: React.ReactNode
  /** The sidebar component to render (e.g. `<AppSidebar ... />`). */
  sidebar: React.ReactNode
  /**
   * Optional global element rendered after the main area. Use for floating
   * UI like a command-palette dialog.
   */
  searchBar?: React.ReactNode

  // ── Toolbar slots ─────────────────────────────────────────────────────────
  /** Toolbar content rendered at the start (left). E.g. breadcrumbs. */
  toolbarLeading?: React.ReactNode
  /** Toolbar content rendered at the end (right). E.g. action buttons. */
  toolbarTrailing?: React.ReactNode
  /**
   * Centred toolbar slot, typically a [`SearchTrigger`]. Constrained to
   * `max-w-md` so it doesn't span the whole bar on wide screens.
   */
  toolbarCenter?: React.ReactNode

  /**
   * Deprecated: pass content via `toolbarLeading` / `toolbarTrailing`
   * / `toolbarCenter` instead.
   */
  toolbarExtra?: React.ReactNode
  /**
   * Deprecated: render via `toolbarTrailing` if you want a kbd hint.
   */
  shortcutHint?: React.ReactNode

  /** Tailwind class for the main content area. Default `"p-4 md:p-6"`. */
  contentClassName?: string
}

export function DashboardShell({
  children,
  sidebar,
  searchBar,
  toolbarLeading,
  toolbarTrailing,
  toolbarCenter,
  toolbarExtra,
  shortcutHint,
  contentClassName = "p-4 md:p-6",
}: DashboardShellProps) {
  return (
    <div data-slot="runesh-dashboard-shell" className="flex h-screen w-full bg-background">
      {sidebar}
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <header className="sticky top-0 z-30 flex h-14 shrink-0 items-center gap-3 border-b border-border bg-background/80 px-4 backdrop-blur supports-[backdrop-filter]:bg-background/60">
          {/* Leading slot */}
          {toolbarLeading && (
            <div className="flex items-center gap-2">{toolbarLeading}</div>
          )}

          {/* Centred slot, capped width so it doesn't bleed to the edges */}
          {toolbarCenter && (
            <div className="flex flex-1 justify-center">
              <div className="w-full max-w-md">{toolbarCenter}</div>
            </div>
          )}
          {!toolbarCenter && <div className="flex-1" />}

          {/* Trailing slot */}
          {toolbarTrailing && (
            <div className="flex items-center gap-2">{toolbarTrailing}</div>
          )}

          {/* Legacy slots, kept for back compat */}
          {toolbarExtra}
          {shortcutHint && !toolbarTrailing && shortcutHint}
        </header>
        <main className={`min-w-0 flex-1 overflow-y-auto ${contentClassName}`}>
          {children}
        </main>
      </div>
      {searchBar}
    </div>
  )
}

// ── SearchTrigger helper ─────────────────────────────────────────────────────

export interface SearchTriggerProps {
  /** Called when the user clicks the trigger or hits the keyboard shortcut. */
  onClick?: () => void
  /** Placeholder text shown inside the trigger. Default `"Search..."`. */
  placeholder?: string
  /**
   * Keyboard shortcut hint shown on the right of the trigger. Default `"⌘K"`
   * on macOS, `"Ctrl K"` elsewhere. Pass an empty string to hide.
   */
  shortcut?: string
  /** Additional className for the trigger button. */
  className?: string
}

/**
 * Search button styled like an input that opens a command palette.
 *
 * The standard pattern used by Linear, Vercel, Cal.com, GitHub: a clickable
 * button in the toolbar that visually looks like a search input with a
 * keyboard shortcut hint, but actually triggers a [`searchBar`] command
 * dialog when clicked. The consumer wires `onClick` to open their dialog
 * (and listens for the same shortcut globally).
 */
export function SearchTrigger({
  onClick,
  placeholder = "Search...",
  shortcut,
  className,
}: SearchTriggerProps) {
  // Default to platform-aware shortcut hint.
  const platformShortcut = React.useMemo(() => {
    if (shortcut !== undefined) return shortcut
    if (typeof navigator === "undefined") return "Ctrl K"
    return /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘K" : "Ctrl K"
  }, [shortcut])

  return (
    <button
      type="button"
      onClick={onClick}
      className={[
        "flex h-9 w-full items-center gap-2 rounded-md border border-input bg-background px-3",
        "text-sm text-muted-foreground",
        "outline-none transition-colors",
        "hover:border-ring hover:text-foreground",
        "focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring",
        className ?? "",
      ].join(" ")}
    >
      <Search className="size-4 shrink-0 opacity-70" />
      <span className="flex-1 truncate text-left">{placeholder}</span>
      {platformShortcut && (
        <kbd className="pointer-events-none hidden h-5 select-none items-center gap-1 rounded border border-border bg-muted px-1.5 font-mono text-[10px] font-medium text-muted-foreground sm:inline-flex">
          {platformShortcut}
        </kbd>
      )}
    </button>
  )
}
