/**
 * Shared font configuration (best implementation from HARUMI-NET).
 *
 * Two ways to use:
 *
 * 1. Google Fonts link tag (static export / SSG / Tauri apps):
 *    Add to <head>:
 *      <link rel="stylesheet"
 *            href="https://fonts.googleapis.com/css2?family=Chiron+GoRound+TC:wght@300;400;500;700&display=swap" />
 *    Then on <body>:
 *      style={{ fontFamily: "'Chiron GoRound TC', system-ui, sans-serif" }}
 *
 * 2. next/font/google (SSR Next.js apps):
 *    import { Chiron_GoRound_TC, Geist_Mono } from "next/font/google"
 *    const chiron = Chiron_GoRound_TC({ variable: "--font-chiron-goround", subsets: ["latin"], display: "swap" })
 *    const mono = Geist_Mono({ variable: "--font-geist-mono", subsets: ["latin"] })
 *    <html className={`${chiron.variable} ${mono.variable}`}>
 *
 * In globals.css, map the font variable:
 *    --font-sans: var(--font-chiron-goround);
 *    --font-mono: var(--font-geist-mono);
 */

/** Google Fonts URL for Chiron GoRound TC with all needed weights */
export const CHIRON_GOROUND_URL =
  "https://fonts.googleapis.com/css2?family=Chiron+GoRound+TC:wght@300;400;500;700&display=swap";

/** Font-family value with system fallbacks */
export const FONT_FAMILY_SANS = "'Chiron GoRound TC', system-ui, sans-serif";
export const FONT_FAMILY_MONO = "'Geist Mono', ui-monospace, monospace";

/** CSS variable names used across projects */
export const FONT_VAR_SANS = "--font-chiron-goround";
export const FONT_VAR_MONO = "--font-geist-mono";
