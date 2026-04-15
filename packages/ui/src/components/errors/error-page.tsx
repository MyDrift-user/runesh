"use client"

/**
 * Standardised RUNESH error pages.
 *
 * Uses shadcn Button and Card from the consumer app, matching the style
 * of the hand-written error pages in RUMMZ and other consumer projects.
 *
 * Covers the well-known HTTP error codes: 400, 401, 403, 404, 500, 502, 503.
 * Consumer can use the generic `ErrorPage` or the named shortcuts
 * (`NotFoundPage`, `ForbiddenPage`, etc.).
 *
 * @example
 * ```tsx
 * // app/not-found.tsx
 * import Link from "next/link"
 * import { NotFoundPage } from "@mydrift/runesh-ui/components/errors/error-page"
 *
 * export default function NotFound() {
 *   return <NotFoundPage homeHref="/" linkComponent={Link} />
 * }
 * ```
 *
 * @example
 * ```tsx
 * // app/error.tsx
 * import { InternalErrorPage } from "@mydrift/runesh-ui/components/errors/error-page"
 *
 * export default function Error({ error, reset }: { error: Error; reset: () => void }) {
 *   return (
 *     <InternalErrorPage
 *       detail={error.message}
 *       onRetry={reset}
 *       homeHref="/"
 *       linkComponent={Link}
 *     />
 *   )
 * }
 * ```
 */

import * as React from "react"
import {
  AlertTriangle,
  ArrowLeft,
  Ban,
  FileQuestion,
  Home,
  Lock,
  RefreshCw,
  ServerCrash,
  ShieldX,
  WifiOff,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import type { LinkLike } from "../layout/app-sidebar"

// ── Types ───────────────────────────────────────────────────────────────────

export type ErrorStatus = 400 | 401 | 403 | 404 | 500 | 502 | 503

export interface ErrorPageProps {
  /** HTTP status code. Drives the default icon, title, and description. */
  status: ErrorStatus
  /** Override the default title for the status code. */
  title?: string
  /** Override the default description. */
  description?: string
  /** Additional detail text rendered below the description (e.g. error message, request ID). */
  detail?: string

  /** When set, renders a "Go home" button linking to this href. */
  homeHref?: string
  /** Label for the home button. Default `"Go home"`. */
  homeLabel?: string
  /** When set, renders a "Go back" button that calls this function. */
  onBack?: () => void
  /** Label for the back button. Default `"Go back"`. */
  backLabel?: string
  /** When set, renders a "Try again" button. */
  onRetry?: () => void
  /** Label for the retry button. Default `"Try again"`. */
  retryLabel?: string

  /** Router link component (e.g. `next/link`). Defaults to `<a>`. */
  linkComponent?: LinkLike
  /** Optional element rendered below the card (e.g. support link). */
  footer?: React.ReactNode
}

// ── Defaults per status code ────────────────────────────────────────────────

interface ErrorDefaults {
  icon: React.FC<{ className?: string }>
  title: string
  description: string
}

const ERROR_DEFAULTS: Record<ErrorStatus, ErrorDefaults> = {
  400: {
    icon: AlertTriangle,
    title: "Bad request",
    description:
      "The server could not understand the request. Please check your input and try again.",
  },
  401: {
    icon: Lock,
    title: "Unauthorized",
    description: "You need to sign in to access this page.",
  },
  403: {
    icon: ShieldX,
    title: "Forbidden",
    description: "You do not have permission to access this resource.",
  },
  404: {
    icon: FileQuestion,
    title: "Page not found",
    description:
      "The page you are looking for does not exist or has been moved.",
  },
  500: {
    icon: ServerCrash,
    title: "Internal server error",
    description:
      "Something went wrong on our end. Please try again later.",
  },
  502: {
    icon: Ban,
    title: "Bad gateway",
    description:
      "The server received an invalid response from an upstream service.",
  },
  503: {
    icon: WifiOff,
    title: "Service unavailable",
    description:
      "The service is temporarily unavailable. Please try again shortly.",
  },
}

// ── Component ───────────────────────────────────────────────────────────────

const DefaultLink: LinkLike = ({ href, children, ...rest }) => (
  <a href={href} {...rest}>
    {children}
  </a>
)

export function ErrorPage({
  status,
  title,
  description,
  detail,
  homeHref,
  homeLabel = "Go home",
  onBack,
  backLabel = "Go back",
  onRetry,
  retryLabel = "Try again",
  linkComponent: Link = DefaultLink,
  footer,
}: ErrorPageProps) {
  const defaults = ERROR_DEFAULTS[status]
  const Icon = defaults.icon

  const resolvedTitle = title ?? defaults.title
  const resolvedDescription = description ?? defaults.description

  const hasActions = Boolean(homeHref || onBack || onRetry)

  return (
    <div className="flex min-h-[60vh] items-center justify-center p-4">
      <Card className="w-full max-w-md text-center">
        <CardHeader>
          <div className="flex flex-col items-center gap-3">
            <Icon className="size-12 text-muted-foreground" />
            <CardTitle className="text-4xl font-bold">{status}</CardTitle>
          </div>
          <CardDescription className="text-base text-balance">
            {resolvedTitle}
          </CardDescription>
          <p className="text-sm text-muted-foreground">
            {resolvedDescription}
          </p>
        </CardHeader>

        {detail && (
          <CardContent>
            <pre className="overflow-auto rounded-md bg-muted p-3 text-xs text-muted-foreground">
              {detail}
            </pre>
          </CardContent>
        )}

        {hasActions && (
          <CardFooter className="justify-center gap-2">
            {onBack && (
              <Button variant="outline" size="sm" onClick={onBack}>
                <ArrowLeft className="size-3.5" data-icon="inline-start" />
                {backLabel}
              </Button>
            )}
            {onRetry && (
              <Button variant="outline" size="sm" onClick={onRetry}>
                <RefreshCw className="size-3.5" data-icon="inline-start" />
                {retryLabel}
              </Button>
            )}
            {homeHref && (
              <Button variant="ghost" size="sm" asChild>
                <Link href={homeHref}>
                  <Home className="size-3.5" data-icon="inline-start" />
                  {homeLabel}
                </Link>
              </Button>
            )}
          </CardFooter>
        )}
      </Card>

      {footer && (
        <p className="mt-6 text-center text-xs text-muted-foreground">
          {footer}
        </p>
      )}
    </div>
  )
}

// ── Named shortcuts ─────────────────────────────────────────────────────────

export type ErrorPageShortcutProps = Omit<ErrorPageProps, "status">

export function BadRequestPage(props: ErrorPageShortcutProps) {
  return <ErrorPage status={400} {...props} />
}

export function UnauthorizedPage(props: ErrorPageShortcutProps) {
  return <ErrorPage status={401} {...props} />
}

export function ForbiddenPage(props: ErrorPageShortcutProps) {
  return <ErrorPage status={403} {...props} />
}

export function NotFoundPage(props: ErrorPageShortcutProps) {
  return <ErrorPage status={404} {...props} />
}

export function InternalErrorPage(props: ErrorPageShortcutProps) {
  return <ErrorPage status={500} {...props} />
}

export function BadGatewayPage(props: ErrorPageShortcutProps) {
  return <ErrorPage status={502} {...props} />
}

export function ServiceUnavailablePage(props: ErrorPageShortcutProps) {
  return <ErrorPage status={503} {...props} />
}
