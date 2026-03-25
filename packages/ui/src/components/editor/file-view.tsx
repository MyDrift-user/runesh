"use client";

import { NodeViewWrapper } from "@tiptap/react";
import { FileText, Download } from "lucide-react";

const FILE_ICONS: Record<string, string> = {
  "application/pdf": "📄",
  "application/zip": "📦",
  "application/x-rar": "📦",
  "application/msword": "📝",
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document": "📝",
  "application/vnd.ms-excel": "📊",
  "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet": "📊",
  "application/vnd.ms-powerpoint": "📽️",
  "application/vnd.openxmlformats-officedocument.presentationml.presentation": "📽️",
};

function formatSize(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const val = bytes / 1024 ** i;
  return `${val < 10 ? val.toFixed(1) : Math.round(val)} ${units[i]}`;
}

export function FileView({ node, selected }: { node: any; selected: boolean }) {
  const { src, fileName, fileSize, fileType } = node.attrs;
  const icon = FILE_ICONS[fileType] || null;
  const isPdf = fileType === "application/pdf";

  return (
    <NodeViewWrapper className="my-4">
      {isPdf ? (
        <div className={`rounded-lg border border-border overflow-hidden ${selected ? "ring-2 ring-primary" : ""}`}>
          <div className="flex items-center gap-2 px-3 py-2 bg-card border-b border-border">
            <span>📄</span>
            <span className="text-sm font-medium truncate flex-1">{fileName}</span>
            <span className="text-xs text-muted-foreground">{formatSize(fileSize)}</span>
            <a href={src} target="_blank" rel="noopener" className="text-xs text-primary hover:underline">Open</a>
          </div>
          <iframe src={src} className="w-full h-96" title={fileName} />
        </div>
      ) : (
        <div className={`flex items-center gap-3 rounded-lg border border-border bg-card p-3 ${selected ? "ring-2 ring-primary" : ""}`}>
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-md bg-muted text-lg">
            {icon || <FileText className="h-5 w-5 text-muted-foreground" />}
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-sm font-medium truncate">{fileName || "File"}</p>
            <p className="text-xs text-muted-foreground">{formatSize(fileSize)}</p>
          </div>
          <a
            href={src}
            download={fileName}
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md hover:bg-accent transition-colors"
          >
            <Download className="h-4 w-4 text-muted-foreground" />
          </a>
        </div>
      )}
    </NodeViewWrapper>
  );
}
