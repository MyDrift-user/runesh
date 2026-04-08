"use client"

/**
 * Standardised RUNESH login screen.
 *
 * Self-contained, no shadcn primitive dependency. Centred card on a muted
 * background with brand mark, title, optional description, username +
 * password fields, optional OIDC button, optional register / forgot links.
 *
 * Consumer wires the actual auth via `onSubmit`. The component manages
 * its own input state but exposes `loading` and `error` for the parent
 * to drive the UX.
 *
 * @example
 * ```tsx
 * <LoginScreen
 *   brandIcon={<Logo />}
 *   brandName="RUMMZ"
 *   description="Sign in to your media server"
 *   loading={isLoading}
 *   error={error?.message}
 *   onSubmit={(creds) => login(creds)}
 *   linkComponent={Link}
 *   registerHref="/login/register"
 * />
 * ```
 */

import * as React from "react"
import { Loader2 } from "lucide-react"
import type { LinkLike } from "../layout/app-sidebar"

export interface LoginCredentials {
  username: string
  password: string
}

export interface LoginScreenProps {
  /** Brand icon, rendered above the title. ~32-48px tall recommended. */
  brandIcon?: React.ReactNode
  /** Brand name, rendered as the screen title. */
  brandName: string
  /** Subheading under the brand. */
  description?: string

  /** Called with the entered credentials when the user submits. */
  onSubmit: (credentials: LoginCredentials) => void | Promise<void>
  /** Disable the form and show a spinner on the submit button. */
  loading?: boolean
  /** Render an inline error banner above the form. */
  error?: string

  /** Username field label. Default `"Username"`. Pass `"Email"` for email auth. */
  usernameLabel?: string
  /** Username field placeholder. */
  usernamePlaceholder?: string

  // ── Optional OIDC button ──────────────────────────────────────────────────
  /** When set, renders an "{ssoLabel}" button above the form that calls onSso. */
  onSso?: () => void
  ssoLabel?: string
  ssoIcon?: React.ReactNode

  // ── Optional links ────────────────────────────────────────────────────────
  /** Router link component (e.g. `next/link`'s `Link`). Defaults to `<a>`. */
  linkComponent?: LinkLike
  /** When set, renders a "Forgot password?" link to this href. */
  forgotHref?: string
  /** When set, renders a "Create account" link below the form. */
  registerHref?: string
  registerLabel?: string

  /** Optional element rendered at the very bottom of the card (e.g. terms link). */
  footer?: React.ReactNode
}

const DefaultLink: LinkLike = ({ href, children, ...rest }) => (
  <a href={href} {...rest}>
    {children}
  </a>
)

export function LoginScreen({
  brandIcon,
  brandName,
  description,
  onSubmit,
  loading = false,
  error,
  usernameLabel = "Username",
  usernamePlaceholder = "Enter your username",
  onSso,
  ssoLabel = "Continue with SSO",
  ssoIcon,
  linkComponent: Link = DefaultLink,
  forgotHref,
  registerHref,
  registerLabel = "Create account",
  footer,
}: LoginScreenProps) {
  const [username, setUsername] = React.useState("")
  const [password, setPassword] = React.useState("")

  function handleSubmit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault()
    if (loading) return
    void onSubmit({ username, password })
  }

  return (
    <AuthShell>
      <AuthCard
        brandIcon={brandIcon}
        brandName={brandName}
        description={description}
      >
        {error && (
          <div
            role="alert"
            className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
          >
            {error}
          </div>
        )}

        {onSso && (
          <>
            <button
              type="button"
              onClick={onSso}
              disabled={loading}
              className={authButton({ variant: "outline" })}
            >
              {ssoIcon}
              <span>{ssoLabel}</span>
            </button>
            <Divider label="or continue with" />
          </>
        )}

        <form onSubmit={handleSubmit} className="space-y-4">
          <Field
            id="username"
            label={usernameLabel}
            type="text"
            autoComplete="username"
            value={username}
            onChange={setUsername}
            placeholder={usernamePlaceholder}
            disabled={loading}
            required
          />

          <Field
            id="password"
            label="Password"
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={setPassword}
            placeholder="Enter your password"
            disabled={loading}
            required
            labelExtra={
              forgotHref && (
                <Link
                  href={forgotHref}
                  className="text-xs font-medium text-muted-foreground hover:text-foreground"
                >
                  Forgot password?
                </Link>
              )
            }
          />

          <button
            type="submit"
            disabled={loading || !username || !password}
            className={authButton({ variant: "primary" })}
          >
            {loading && <Loader2 className="size-4 animate-spin" />}
            <span>Sign in</span>
          </button>
        </form>

        {registerHref && (
          <p className="text-center text-sm text-muted-foreground">
            Don&apos;t have an account?{" "}
            <Link
              href={registerHref}
              className="font-medium text-foreground hover:underline"
            >
              {registerLabel}
            </Link>
          </p>
        )}
      </AuthCard>
      {footer && (
        <p className="mt-6 text-center text-xs text-muted-foreground">{footer}</p>
      )}
    </AuthShell>
  )
}

// ── Shared shell + primitives (re-used by SetupScreen) ──────────────────────

export function AuthShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex min-h-screen items-center justify-center bg-muted/30 px-4 py-12">
      <div className="w-full max-w-sm">{children}</div>
    </div>
  )
}

export interface AuthCardProps {
  brandIcon?: React.ReactNode
  brandName: string
  description?: string
  children: React.ReactNode
}

export function AuthCard({
  brandIcon,
  brandName,
  description,
  children,
}: AuthCardProps) {
  return (
    <div className="rounded-2xl border border-border bg-card p-8 shadow-sm">
      <div className="mb-6 flex flex-col items-center gap-3 text-center">
        {brandIcon && <div className="flex items-center justify-center">{brandIcon}</div>}
        <h1 className="text-xl font-semibold tracking-tight text-foreground">
          {brandName}
        </h1>
        {description && (
          <p className="text-sm text-muted-foreground">{description}</p>
        )}
      </div>
      <div className="space-y-4">{children}</div>
    </div>
  )
}

interface FieldProps {
  id: string
  label: string
  type: string
  value: string
  onChange: (value: string) => void
  placeholder?: string
  autoComplete?: string
  disabled?: boolean
  required?: boolean
  labelExtra?: React.ReactNode
}

function Field({
  id,
  label,
  type,
  value,
  onChange,
  placeholder,
  autoComplete,
  disabled,
  required,
  labelExtra,
}: FieldProps) {
  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between">
        <label
          htmlFor={id}
          className="text-sm font-medium leading-none text-foreground"
        >
          {label}
        </label>
        {labelExtra}
      </div>
      <input
        id={id}
        type={type}
        autoComplete={autoComplete}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        disabled={disabled}
        required={required}
        className="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none transition-colors placeholder:text-muted-foreground hover:border-ring focus:border-ring focus:ring-2 focus:ring-ring/30 disabled:cursor-not-allowed disabled:opacity-50"
      />
    </div>
  )
}

function Divider({ label }: { label: string }) {
  return (
    <div className="relative">
      <div className="absolute inset-0 flex items-center">
        <div className="w-full border-t border-border" />
      </div>
      <div className="relative flex justify-center text-xs">
        <span className="bg-card px-2 text-muted-foreground">{label}</span>
      </div>
    </div>
  )
}

function authButton({ variant }: { variant: "primary" | "outline" }) {
  const base =
    "inline-flex h-10 w-full items-center justify-center gap-2 rounded-md px-4 text-sm font-medium outline-none transition-colors disabled:cursor-not-allowed disabled:opacity-50 focus-visible:ring-2 focus-visible:ring-ring"
  if (variant === "primary") {
    return `${base} bg-primary text-primary-foreground hover:bg-primary/90`
  }
  return `${base} border border-input bg-background hover:bg-accent hover:text-accent-foreground`
}

// Re-export the field primitives so SetupScreen can use them.
export { Field as AuthField, authButton as authButtonClass }
