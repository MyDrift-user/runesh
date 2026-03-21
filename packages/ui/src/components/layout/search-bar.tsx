"use client"

import { useEffect, useState, useCallback } from "react"
import { useRouter } from "next/navigation"
import {
  CommandDialog,
  CommandInput,
  CommandList,
  CommandEmpty,
  CommandGroup,
  CommandItem,
} from "@/components/ui/command"
import { Loader2, type LucideIcon } from "lucide-react"

export interface SearchResult {
  id: string
  title: string
  href: string
  icon?: LucideIcon
  subtitle?: string
  group: string
}

export interface QuickLink {
  title: string
  href: string
  icon: LucideIcon
}

export interface SearchBarProps {
  /** Called when the user types a search query. Return results grouped by category. */
  onSearch: (query: string) => Promise<SearchResult[]>
  /** Quick navigation links always shown at the bottom */
  quickLinks?: QuickLink[]
  /** Debounce delay in ms (default: 300) */
  debounceMs?: number
  /** Placeholder text for the search input */
  placeholder?: string
}

export function SearchBar({
  onSearch,
  quickLinks = [],
  debounceMs = 300,
  placeholder = "Search...",
}: SearchBarProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState("")
  const [results, setResults] = useState<SearchResult[]>([])
  const [loading, setLoading] = useState(false)
  const router = useRouter()

  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault()
        setOpen((prev) => !prev)
      }
    }
    document.addEventListener("keydown", down)
    return () => document.removeEventListener("keydown", down)
  }, [])

  const search = useCallback(async (q: string) => {
    if (q.length < 2) {
      setResults([])
      return
    }
    setLoading(true)
    try {
      const r = await onSearch(q)
      setResults(r)
    } catch {
      // silently fail search
    } finally {
      setLoading(false)
    }
  }, [onSearch])

  useEffect(() => {
    const timer = setTimeout(() => {
      search(query)
    }, debounceMs)
    return () => clearTimeout(timer)
  }, [query, search, debounceMs])

  const handleSelect = (href: string) => {
    router.push(href)
    setOpen(false)
    setQuery("")
    setResults([])
  }

  // Group results by their group field
  const grouped = results.reduce<Record<string, SearchResult[]>>((acc, r) => {
    if (!acc[r.group]) acc[r.group] = []
    acc[r.group].push(r)
    return acc
  }, {})

  return (
    <CommandDialog
      open={open}
      onOpenChange={(v) => {
        setOpen(v)
        if (!v) {
          setQuery("")
          setResults([])
        }
      }}
      title="Search"
      description="Search and navigate"
    >
      <CommandInput
        placeholder={placeholder}
        value={query}
        onValueChange={setQuery}
      />
      <CommandList>
        <CommandEmpty>
          {loading ? (
            <div className="flex items-center justify-center gap-2 py-4">
              <Loader2 className="h-4 w-4 animate-spin" />
              <span>Searching...</span>
            </div>
          ) : (
            "No results found."
          )}
        </CommandEmpty>

        {Object.entries(grouped).map(([group, items]) => (
          <CommandGroup key={group} heading={group}>
            {items.map((item) => (
              <CommandItem
                key={item.id}
                onSelect={() => handleSelect(item.href)}
              >
                {item.icon && <item.icon className="mr-2 h-4 w-4" />}
                <span>{item.title}</span>
                {item.subtitle && (
                  <span className="ml-auto text-xs text-muted-foreground">{item.subtitle}</span>
                )}
              </CommandItem>
            ))}
          </CommandGroup>
        ))}

        {quickLinks.length > 0 && (
          <CommandGroup heading="Quick Links">
            {quickLinks.map((link) => (
              <CommandItem
                key={link.href}
                onSelect={() => handleSelect(link.href)}
              >
                <link.icon className="mr-2 h-4 w-4" />
                <span>{link.title}</span>
              </CommandItem>
            ))}
          </CommandGroup>
        )}
      </CommandList>
    </CommandDialog>
  )
}
