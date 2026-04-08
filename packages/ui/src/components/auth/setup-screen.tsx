"use client"

/**
 * Standardised RUNESH first-run setup wizard.
 *
 * Same visual shell as [`LoginScreen`] (centred card, brand, muted bg) so
 * the unauthenticated screens feel like one piece. Multi-step content is
 * passed in as an array of steps; the component renders a header with
 * step indicator, the active step's body, and a footer with Back / Next
 * buttons.
 *
 * Steps are fully owned by the consumer (each renders its own form,
 * validation, etc.). The wizard only handles the chrome and the
 * forward/backward navigation.
 *
 * @example
 * ```tsx
 * <SetupScreen
 *   brandIcon={<Logo />}
 *   brandName="RUMMZ"
 *   description="First-run setup"
 *   steps={[
 *     { title: "Admin", body: <AdminForm /> },
 *     { title: "TMDB", body: <TmdbForm />, canAdvance: tmdbValid },
 *     { title: "Indexers", body: <IndexerForm /> },
 *   ]}
 *   onComplete={() => router.push("/")}
 * />
 * ```
 */

import * as React from "react"
import { Check, ChevronLeft, ChevronRight, Loader2 } from "lucide-react"
import { AuthCard, AuthShell, authButtonClass } from "./login-screen"

export interface SetupStep {
  /** Step title shown in the indicator and the card header. */
  title: string
  /** Optional one-line description shown under the title. */
  description?: string
  /** The step's actual content (form, info text, etc.). */
  body: React.ReactNode
  /**
   * Set to false to disable the Next button (e.g. form invalid). Defaults
   * to true.
   */
  canAdvance?: boolean
  /**
   * Called when the user clicks Next. Return a Promise that rejects to
   * cancel the advance. Use for save-on-next semantics.
   */
  onNext?: () => void | Promise<void>
}

export interface SetupScreenProps {
  brandIcon?: React.ReactNode
  brandName: string
  description?: string

  /** Ordered list of wizard steps. */
  steps: SetupStep[]
  /** Index of the active step. If omitted, the wizard manages it internally. */
  step?: number
  /** Called when the active step changes (controlled mode). */
  onStepChange?: (step: number) => void

  /** Called when the user clicks Finish on the last step. */
  onComplete: () => void | Promise<void>

  /** Disable both Back and Next, show spinner on Next. */
  loading?: boolean
  /** Inline error rendered above the buttons. */
  error?: string

  /** Optional element rendered at the very bottom of the card. */
  footer?: React.ReactNode
}

export function SetupScreen({
  brandIcon,
  brandName,
  description,
  steps,
  step: stepProp,
  onStepChange,
  onComplete,
  loading = false,
  error,
  footer,
}: SetupScreenProps) {
  const [internalStep, setInternalStep] = React.useState(0)
  const isControlled = stepProp !== undefined
  const step = isControlled ? stepProp! : internalStep
  const setStep = React.useCallback(
    (next: number) => {
      const clamped = Math.max(0, Math.min(steps.length - 1, next))
      if (isControlled) onStepChange?.(clamped)
      else setInternalStep(clamped)
    },
    [isControlled, onStepChange, steps.length]
  )

  const current = steps[step]
  const isFirst = step === 0
  const isLast = step === steps.length - 1
  const canAdvance = current?.canAdvance !== false

  async function handleNext() {
    if (loading || !canAdvance) return
    try {
      if (current?.onNext) await current.onNext()
    } catch {
      return
    }
    if (isLast) {
      await onComplete()
    } else {
      setStep(step + 1)
    }
  }

  return (
    <AuthShell>
      <AuthCard
        brandIcon={brandIcon}
        brandName={brandName}
        description={description}
      >
        <StepIndicator total={steps.length} current={step} titles={steps.map((s) => s.title)} />

        <div className="space-y-1">
          <h2 className="text-base font-semibold tracking-tight text-foreground">
            {current?.title}
          </h2>
          {current?.description && (
            <p className="text-sm text-muted-foreground">{current.description}</p>
          )}
        </div>

        <div className="min-h-[120px]">{current?.body}</div>

        {error && (
          <div
            role="alert"
            className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
          >
            {error}
          </div>
        )}

        <div className="flex items-center gap-2 pt-2">
          <button
            type="button"
            onClick={() => setStep(step - 1)}
            disabled={loading || isFirst}
            className={authButtonClass({ variant: "outline" })}
          >
            <ChevronLeft className="size-4" />
            <span>Back</span>
          </button>
          <button
            type="button"
            onClick={handleNext}
            disabled={loading || !canAdvance}
            className={authButtonClass({ variant: "primary" })}
          >
            {loading && <Loader2 className="size-4 animate-spin" />}
            <span>{isLast ? "Finish" : "Next"}</span>
            {!isLast && !loading && <ChevronRight className="size-4" />}
          </button>
        </div>
      </AuthCard>
      {footer && (
        <p className="mt-6 text-center text-xs text-muted-foreground">{footer}</p>
      )}
    </AuthShell>
  )
}

// ── Step indicator ──────────────────────────────────────────────────────────

function StepIndicator({
  total,
  current,
  titles,
}: {
  total: number
  current: number
  titles: string[]
}) {
  return (
    <ol className="flex items-center gap-1.5" aria-label="Setup progress">
      {Array.from({ length: total }).map((_, i) => {
        const state =
          i < current ? "complete" : i === current ? "active" : "pending"
        return (
          <li key={i} className="flex flex-1 items-center gap-1.5" aria-current={state === "active" ? "step" : undefined}>
            <div
              className={[
                "flex h-6 w-6 shrink-0 items-center justify-center rounded-full border text-[11px] font-medium transition-colors",
                state === "complete" && "border-primary bg-primary text-primary-foreground",
                state === "active" && "border-foreground bg-background text-foreground",
                state === "pending" && "border-border bg-background text-muted-foreground",
              ]
                .filter(Boolean)
                .join(" ")}
            >
              {state === "complete" ? <Check className="size-3" /> : i + 1}
            </div>
            {i < total - 1 && (
              <div
                className={[
                  "h-px flex-1 transition-colors",
                  i < current ? "bg-primary" : "bg-border",
                ].join(" ")}
              />
            )}
          </li>
        )
      })}
    </ol>
  )
}
