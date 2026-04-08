"use client"

/**
 * Theme-aware wrapper around `auto-skeleton-react`.
 *
 * The upstream package walks the rendered DOM of `children`, measures every
 * element (rect, computed style, tag, text content), classifies them as
 * text / image / button / input / container / etc., and renders a placeholder
 * tree of the same shape while `loading` is true. Result: no manual skeleton
 * authoring for ~80% of pages.
 *
 * This wrapper:
 *   - Defaults to a `shimmer` animation with theme-aware base/highlight
 *     colours pulled from the consumer's CSS variables (so light + dark
 *     mode both look right without extra config).
 *   - Re-exports the upstream `SkeletonConfig` type for callers that want
 *     to override anything else.
 *   - Adds a `noSkeletonAttr` helper to spread on elements that should
 *     render normally even while loading (e.g. brand logos in the loading
 *     state of a header).
 *
 * Opt-out of skeletonisation for a single element by setting
 * `data-no-skeleton` (or className `no-skeleton`) on it. Use the
 * `noSkeletonAttr()` helper for type safety.
 */

import * as React from "react"
import { AutoSkeleton as UpstreamAutoSkeleton } from "auto-skeleton-react"
import type { SkeletonConfig } from "auto-skeleton-react"

export type { SkeletonConfig }

export interface AutoSkeletonProps {
  /** When true, render the auto-generated skeleton instead of children. */
  loading: boolean
  /** Real content. The shape of this is what the skeleton mirrors. */
  children: React.ReactNode
  /** Override any default config field. */
  config?: Partial<SkeletonConfig>
}

/**
 * Default config tuned for shadcn-style themes. Reads `--muted` and
 * `--accent` from the document root so it tracks light/dark mode.
 *
 * If those CSS vars don't exist on the consumer (very unlikely with
 * shadcn) the upstream defaults take over.
 */
function getThemeDefaults(): Partial<SkeletonConfig> {
  if (typeof window === "undefined") {
    // SSR fallback. The actual skeleton runs on the client anyway, but
    // be defensive so this can render in any context.
    return { animation: "shimmer" }
  }
  const root = getComputedStyle(document.documentElement)
  const muted = root.getPropertyValue("--muted").trim()
  const accent = root.getPropertyValue("--accent").trim()
  return {
    animation: "shimmer",
    baseColor: muted ? `oklch(from ${muted} l c h / 0.5)` : "var(--muted)",
    highlightColor: accent ? `oklch(from ${accent} l c h / 0.4)` : "var(--accent)",
    borderRadius: 6,
  }
}

export function AutoSkeleton({ loading, children, config }: AutoSkeletonProps) {
  // Recompute defaults whenever the resolved theme changes. We listen on
  // the html class attribute (next-themes flips between `light` / `dark`)
  // so the skeleton swaps colours without a remount.
  const [theme, setTheme] = React.useState(0)
  React.useEffect(() => {
    if (typeof window === "undefined") return
    const observer = new MutationObserver(() => setTheme((n) => n + 1))
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    })
    return () => observer.disconnect()
  }, [])

  const merged = React.useMemo<Partial<SkeletonConfig>>(
    () => ({ ...getThemeDefaults(), ...config }),
    // theme is in the deps so a className flip recomputes the colours
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [config, theme]
  )

  return (
    <UpstreamAutoSkeleton loading={loading} config={merged}>
      {children}
    </UpstreamAutoSkeleton>
  )
}

/**
 * Spread on any element that should render normally even when its
 * ancestor `<AutoSkeleton loading>` is active. Use for brand marks,
 * navigation chrome, or anything where the placeholder shape would
 * be misleading.
 *
 * @example
 * <header {...noSkeletonAttr()}>
 *   <BrandLogo />
 * </header>
 */
export function noSkeletonAttr() {
  return { "data-no-skeleton": true } as const
}
