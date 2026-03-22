"use client";

import { useState, useEffect } from "react";

/** Returns true if the app is running inside a Tauri webview. */
export function isTauri(): boolean {
  if (typeof window === "undefined") return false;
  return "__TAURI_INTERNALS__" in window;
}

/** React hook that returns whether the app is running in Tauri. */
export function useTauri() {
  const [inTauri, setInTauri] = useState(false);

  useEffect(() => {
    setInTauri(isTauri());
  }, []);

  return inTauri;
}
