"use client";

import { NodeViewWrapper } from "@tiptap/react";
import { FileText, FileSpreadsheet, FileImage, FileVideo, FileArchive, Presentation, Download, ExternalLink } from "lucide-react";
import type { LucideIcon } from "lucide-react";

function getFileIcon(mimeType: string): LucideIcon {
  if (mimeType.startsWith("image/")) return FileImage;
  if (mimeType.startsWith("video/")) return FileVideo;
  if (mimeType.includes("spreadsheet") || mimeType.includes("excel") || mimeType === "text/csv") return FileSpreadsheet;
  if (mimeType.includes("presentation") || mimeType.includes("powerpoint")) return Presentation;
  if (mimeType.includes("zip") || mimeType.includes("rar") || mimeType.includes("tar") || mimeType.includes("gzip")) return FileArchive;
  return FileText;
}

function formatSize(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const val = bytes / 1024 ** i;
  return `${val < 10 ? val.toFixed(1) : Math.round(val)} ${units[i]}`;
}

export function FileView({ node, selected }: { node: any; selected: boolean }) {
  const { src, fileName, fileSize, fileType } = node.attrs;
  const isPdf = fileType === "application/pdf";
  const Icon = getFileIcon(fileType);

  if (isPdf) {
    return (
      <NodeViewWrapper className="my-3 not-prose">
        <div className={`rounded-lg border overflow-hidden transition-colors ${selected ? "ring-2 ring-primary border-primary" : "border-border"}`}>
          <div className="group flex items-center gap-2 px-3 py-2 bg-card border-b border-border">
            <FileText className="h-4 w-4 text-red-500 shrink-0" />
            <span className="text-sm font-medium truncate flex-1">{fileName}</span>
            <span className="text-xs text-muted-foreground">{formatSize(fileSize)}</span>
            <a href={src} target="_blank" rel="noopener" className="flex h-6 w-6 items-center justify-center rounded-md hover:bg-accent transition-colors" title="Open in new tab">
              <ExternalLink className="h-3.5 w-3.5 text-muted-foreground" />
            </a>
            <a href={src} download={fileName} className="flex h-6 w-6 items-center justify-center rounded-md hover:bg-accent transition-colors" title="Download">
              <Download className="h-3.5 w-3.5 text-muted-foreground" />
            </a>
          </div>
          <iframe src={src} className="w-full h-96 bg-muted/10" title={fileName} />
        </div>
      </NodeViewWrapper>
    );
  }

  return (
    <NodeViewWrapper className="my-3 not-prose">
      <div className={`group flex items-center gap-3 rounded-lg border bg-card px-4 py-3 transition-colors ${selected ? "ring-2 ring-primary border-primary" : "border-border hover:border-muted-foreground/30"}`}>
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted">
          <Icon className="h-4 w-4 text-muted-foreground" />
        </div>
        <div className="flex-1 min-w-0">
          <p className="text-sm font-medium truncate leading-none">{fileName || "File"}</p>
          <p className="text-xs text-muted-foreground mt-0.5">{formatSize(fileSize)}</p>
        </div>
        <a href={src} download={fileName} className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md opacity-0 group-hover:opacity-100 hover:bg-accent transition-all" title="Download">
          <Download className="h-4 w-4 text-muted-foreground" />
        </a>
      </div>
    </NodeViewWrapper>
  );
}
