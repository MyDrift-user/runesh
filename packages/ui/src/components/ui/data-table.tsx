"use client";

import * as React from "react";
import { ChevronUpIcon, ChevronDownIcon, ChevronsUpDownIcon, SearchIcon } from "lucide-react";
import { cn } from "../../lib/utils";
import { Input } from "./input";
import { Button } from "./button";
import { Skeleton } from "./skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "./select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./table";

export interface DataTableColumn<T> {
  key: string;
  header: string;
  getValue: (row: T) => string | number;
  renderCell?: (row: T) => React.ReactNode;
  className?: string;
  sortable?: boolean;
}

export interface ServerPagination {
  total: number | null;
  page: number;
  pageSize: number;
  onPageChange: (page: number) => void;
  onPageSizeChange: (size: number) => void;
}

export interface DataTableProps<T extends { id: string }> {
  columns: DataTableColumn<T>[];
  data: T[];
  loading?: boolean;
  skeletonRows?: number;
  renderRowActions?: (row: T) => React.ReactNode;
  emptyMessage?: string;
  searchPlaceholder?: string;
  hideSearch?: boolean;
  renderFilters?: () => React.ReactNode;
  serverPagination?: ServerPagination;
  defaultPageSize?: number;
  className?: string;
}

const PAGE_SIZES = [25, 50, 100];
type SortDir = "asc" | "desc" | null;

export function DataTable<T extends { id: string }>({
  columns,
  data,
  loading = false,
  skeletonRows = 5,
  renderRowActions,
  emptyMessage = "No results found.",
  searchPlaceholder = "Search...",
  hideSearch = false,
  renderFilters,
  serverPagination,
  defaultPageSize = 25,
  className,
}: DataTableProps<T>) {
  const [search, setSearch] = React.useState("");
  const [sortKey, setSortKey] = React.useState("");
  const [sortDir, setSortDir] = React.useState<SortDir>(null);
  const [clientPage, setClientPage] = React.useState(0);
  const [clientPageSize, setClientPageSize] = React.useState(defaultPageSize);

  React.useEffect(() => {
    if (!serverPagination) setClientPage(0);
  }, [search, serverPagination]);

  const filtered = React.useMemo(() => {
    if (serverPagination || !search.trim()) return data;
    const q = search.toLowerCase();
    return data.filter((row) =>
      columns.some((col) => String(col.getValue(row)).toLowerCase().includes(q))
    );
  }, [data, search, columns, serverPagination]);

  const sorted = React.useMemo(() => {
    if (serverPagination || !sortKey || !sortDir) return filtered;
    const col = columns.find((c) => c.key === sortKey);
    if (!col) return filtered;
    return [...filtered].sort((a, b) => {
      const av = col.getValue(a);
      const bv = col.getValue(b);
      const cmp =
        typeof av === "number" && typeof bv === "number"
          ? av - bv
          : String(av).localeCompare(String(bv), undefined, { sensitivity: "base" });
      return sortDir === "asc" ? cmp : -cmp;
    });
  }, [filtered, sortKey, sortDir, columns, serverPagination]);

  const pageSize = serverPagination ? serverPagination.pageSize : clientPageSize;
  const page = serverPagination ? serverPagination.page : clientPage;
  const pageRows = serverPagination ? sorted : sorted.slice(page * pageSize, page * pageSize + pageSize);
  const totalRows = serverPagination ? serverPagination.total : filtered.length;
  const totalPages = totalRows !== null ? Math.max(1, Math.ceil(totalRows / pageSize)) : null;
  const hasNext = totalPages !== null ? page < totalPages - 1 : pageRows.length === pageSize;

  function toggleSort(key: string) {
    if (serverPagination) return;
    if (sortKey !== key) { setSortKey(key); setSortDir("asc"); }
    else if (sortDir === "asc") setSortDir("desc");
    else { setSortKey(""); setSortDir(null); }
  }

  function goTo(p: number) {
    serverPagination ? serverPagination.onPageChange(p) : setClientPage(p);
  }

  function setSize(s: number) {
    if (serverPagination) serverPagination.onPageSizeChange(s);
    else { setClientPageSize(s); setClientPage(0); }
  }

  const colCount = columns.length + (renderRowActions ? 1 : 0);

  return (
    <div className={cn("space-y-2", className)}>
      {(!hideSearch || renderFilters) && (
        <div className="flex items-center gap-2 flex-wrap">
          {!hideSearch && (
            <div className="relative flex-1 min-w-48 max-w-sm">
              <SearchIcon className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 size-3.5 text-muted-foreground" />
              <Input
                placeholder={searchPlaceholder}
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                className="pl-8"
              />
            </div>
          )}
          {renderFilters?.()}
        </div>
      )}

      <div className="rounded-lg border overflow-hidden">
        <Table>
          <TableHeader>
            <TableRow>
              {columns.map((col) => {
                const canSort = col.sortable !== false && !serverPagination;
                return (
                  <TableHead
                    key={col.key}
                    className={cn(col.className, canSort && "cursor-pointer select-none hover:text-foreground")}
                    onClick={canSort ? () => toggleSort(col.key) : undefined}
                  >
                    {col.header}
                    {canSort && (
                      sortKey === col.key && sortDir
                        ? sortDir === "asc" ? <ChevronUpIcon className="ml-1 inline size-3" /> : <ChevronDownIcon className="ml-1 inline size-3" />
                        : <ChevronsUpDownIcon className="ml-1 inline size-3 text-muted-foreground/40" />
                    )}
                  </TableHead>
                );
              })}
              {renderRowActions && <TableHead className="w-12" />}
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              Array.from({ length: skeletonRows }).map((_, i) => (
                <TableRow key={i}>
                  {columns.map((col) => (
                    <TableCell key={col.key} className={col.className}>
                      <Skeleton className="h-4 w-full" />
                    </TableCell>
                  ))}
                  {renderRowActions && <TableCell className="w-12"><Skeleton className="h-4 w-6" /></TableCell>}
                </TableRow>
              ))
            ) : pageRows.length === 0 ? (
              <TableRow>
                <TableCell colSpan={colCount} className="py-12 text-center text-sm text-muted-foreground">
                  {search && !serverPagination ? `No results for "${search}".` : emptyMessage}
                </TableCell>
              </TableRow>
            ) : (
              pageRows.map((row) => (
                <TableRow key={row.id}>
                  {columns.map((col) => (
                    <TableCell key={col.key} className={col.className}>
                      {col.renderCell ? col.renderCell(row) : String(col.getValue(row))}
                    </TableCell>
                  ))}
                  {renderRowActions && (
                    <TableCell className="w-12 text-right">{renderRowActions(row)}</TableCell>
                  )}
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </div>

      {!loading && (filtered.length > 0 || serverPagination) && (
        <div className="flex items-center justify-between text-sm">
          <div className="flex items-center gap-2 text-muted-foreground">
            <span>Rows</span>
            <Select value={String(pageSize)} onValueChange={(v) => v && setSize(Number(v))}>
              <SelectTrigger size="sm" className="w-16"><SelectValue /></SelectTrigger>
              <SelectContent>
                {PAGE_SIZES.map((n) => <SelectItem key={n} value={String(n)}>{n}</SelectItem>)}
              </SelectContent>
            </Select>
          </div>
          <div className="flex items-center gap-3">
            <span className="text-muted-foreground">
              {totalRows !== null && totalRows > 0
                ? `${page * pageSize + 1}--${Math.min(page * pageSize + pageRows.length, totalRows)} of ${totalRows}`
                : serverPagination ? `Page ${page + 1}` : ""}
            </span>
            <div className="flex gap-1">
              <Button variant="outline" size="sm" disabled={page === 0} onClick={() => goTo(page - 1)}>Prev</Button>
              <Button variant="outline" size="sm" disabled={!hasNext} onClick={() => goTo(page + 1)}>Next</Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
