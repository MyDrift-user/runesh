"use client"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { useState } from "react"

export function QueryProvider({
  children,
  staleTime = 60_000,
  retry = 1,
}: {
  children: React.ReactNode
  staleTime?: number
  retry?: number
}) {
  const [queryClient] = useState(() => new QueryClient({
    defaultOptions: { queries: { staleTime, retry } }
  }))
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
}
