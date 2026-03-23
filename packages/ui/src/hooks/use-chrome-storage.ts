"use client";

import { useEffect, useState, useCallback } from "react";

/**
 * React hook for Chrome extension storage API.
 * Syncs state with chrome.storage and listens for external changes
 * (e.g. from background worker or other tabs).
 *
 * Usage:
 * ```tsx
 * const [theme, setTheme] = useChromeStorage("theme", "dark");
 * const [apiKey, setApiKey] = useChromeStorage("api_key", "", "local");
 * ```
 */
export function useChromeStorage<T>(
  key: string,
  initialValue: T,
  area: "local" | "sync" | "session" = "sync",
): [T, (value: T | ((prev: T) => T)) => void] {
  const [value, setValue] = useState<T>(initialValue);

  useEffect(() => {
    if (typeof chrome === "undefined" || !chrome.storage) return;

    // Load initial value
    chrome.storage[area].get(key, (result) => {
      if (result[key] !== undefined) {
        setValue(result[key]);
      }
    });

    // Listen for external changes
    const listener = (
      changes: { [key: string]: chrome.storage.StorageChange },
      areaName: string,
    ) => {
      if (areaName === area && changes[key]) {
        setValue(changes[key].newValue);
      }
    };

    chrome.storage.onChanged.addListener(listener);
    return () => chrome.storage.onChanged.removeListener(listener);
  }, [key, area]);

  const set = useCallback(
    (newValue: T | ((prev: T) => T)) => {
      setValue((prev) => {
        const resolved = typeof newValue === "function"
          ? (newValue as (prev: T) => T)(prev)
          : newValue;
        if (typeof chrome !== "undefined" && chrome.storage) {
          chrome.storage[area].set({ [key]: resolved });
        }
        return resolved;
      });
    },
    [key, area],
  );

  return [value, set];
}
