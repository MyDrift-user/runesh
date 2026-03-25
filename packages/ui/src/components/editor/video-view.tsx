"use client";

import { NodeViewWrapper } from "@tiptap/react";
import { Film, Download } from "lucide-react";

export function VideoView({ node, selected }: { node: any; selected: boolean }) {
  return (
    <NodeViewWrapper className="my-3 not-prose">
      <div className={`rounded-lg overflow-hidden border transition-colors ${selected ? "ring-2 ring-primary border-primary" : "border-border"}`}>
        <video src={node.attrs.src} controls preload="metadata" className="w-full bg-black" />
        {node.attrs.fileName && (
          <div className="group flex items-center gap-2 px-3 py-2 bg-card border-t border-border">
            <Film className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            <span className="text-xs text-muted-foreground truncate flex-1">{node.attrs.fileName}</span>
            <a href={node.attrs.src} download={node.attrs.fileName} className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md opacity-0 group-hover:opacity-100 hover:bg-accent transition-all" title="Download">
              <Download className="h-3 w-3 text-muted-foreground" />
            </a>
          </div>
        )}
      </div>
    </NodeViewWrapper>
  );
}
