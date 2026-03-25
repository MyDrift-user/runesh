"use client";

import { NodeViewWrapper } from "@tiptap/react";
import { Volume2, Download } from "lucide-react";

export function AudioView({ node, selected }: { node: any; selected: boolean }) {
  return (
    <NodeViewWrapper className="my-3 not-prose">
      <div className={`group flex items-center gap-3 rounded-lg border bg-card px-4 py-3 transition-colors ${selected ? "ring-2 ring-primary border-primary" : "border-border hover:border-muted-foreground/30"}`}>
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-primary/10">
          <Volume2 className="h-4 w-4 text-primary" />
        </div>
        <div className="flex-1 min-w-0 space-y-1">
          <p className="text-sm font-medium truncate leading-none">{node.attrs.fileName || "Audio"}</p>
          <audio src={node.attrs.src} controls preload="metadata" className="w-full h-7" />
        </div>
        <a href={node.attrs.src} download={node.attrs.fileName} className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md opacity-0 group-hover:opacity-100 hover:bg-accent transition-all" title="Download">
          <Download className="h-3.5 w-3.5 text-muted-foreground" />
        </a>
      </div>
    </NodeViewWrapper>
  );
}
