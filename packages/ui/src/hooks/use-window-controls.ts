"use client";

/**
 * Window control hooks for Tauri v2 desktop apps.
 *
 * Usage:
 * ```tsx
 * const { minimize, maximize, close, toggleMaximize } = useWindowControls();
 * ```
 */

let windowModule: typeof import("@tauri-apps/api/webviewWindow") | null = null;

async function getWindow() {
  if (!windowModule) {
    try {
      windowModule = await import("@tauri-apps/api/webviewWindow");
    } catch {
      return null;
    }
  }
  return windowModule.getCurrentWebviewWindow();
}

export function useWindowControls() {
  return {
    minimize: async () => {
      const win = await getWindow();
      await win?.minimize();
    },
    maximize: async () => {
      const win = await getWindow();
      await win?.maximize();
    },
    close: async () => {
      const win = await getWindow();
      await win?.close();
    },
    toggleMaximize: async () => {
      const win = await getWindow();
      await win?.toggleMaximize();
    },
    setAlwaysOnTop: async (onTop: boolean) => {
      const win = await getWindow();
      await win?.setAlwaysOnTop(onTop);
    },
  };
}
