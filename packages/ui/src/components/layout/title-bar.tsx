"use client";

import { Minus, Square, X } from "lucide-react";
import { useWindowControls } from "../../hooks/use-window-controls";
import { cn } from "../../lib/utils";

interface TitleBarProps {
  /** App title displayed in the title bar */
  title: string;
  /** Optional icon element */
  icon?: React.ReactNode;
  /** Additional content in the title bar (e.g. navigation) */
  children?: React.ReactNode;
  /** CSS class for the title bar container */
  className?: string;
}

/**
 * Custom window title bar for Tauri v2 apps with frameless windows.
 *
 * Uses `data-tauri-drag-region` for native window dragging.
 * Provides minimize, maximize, and close buttons.
 *
 * In your tauri.conf.json, set `"decorations": false` to use this.
 */
export function TitleBar({ title, icon, children, className }: TitleBarProps) {
  const { minimize, toggleMaximize, close } = useWindowControls();

  const btnClass =
    "inline-flex items-center justify-center w-11 h-8 hover:bg-accent transition-colors";

  return (
    <div
      data-tauri-drag-region
      className={cn(
        "flex items-center h-8 select-none bg-background border-b shrink-0",
        className,
      )}
    >
      {/* App icon + title */}
      <div className="flex items-center gap-2 px-3 pointer-events-none">
        {icon}
        <span className="text-xs font-medium text-muted-foreground">{title}</span>
      </div>

      {/* Custom content */}
      {children && (
        <div className="flex-1 flex items-center pointer-events-auto">
          {children}
        </div>
      )}

      <div className="flex-1" data-tauri-drag-region />

      {/* Window controls */}
      <div className="flex">
        <button type="button" className={btnClass} onClick={minimize}>
          <Minus className="h-3 w-3" />
        </button>
        <button type="button" className={btnClass} onClick={toggleMaximize}>
          <Square className="h-2.5 w-2.5" />
        </button>
        <button
          type="button"
          className={cn(btnClass, "hover:bg-destructive hover:text-destructive-foreground")}
          onClick={close}
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>
    </div>
  );
}
