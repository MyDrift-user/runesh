"use client";

import { NodeViewWrapper } from "@tiptap/react";
import { Music } from "lucide-react";

export function AudioView({ node, selected }: { node: any; selected: boolean }) {
  return (
    <NodeViewWrapper className="my-4">
      <div className={`flex items-center gap-3 rounded-lg border border-border bg-card p-3 ${selected ? "ring-2 ring-primary" : ""}`}>
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
          <Music className="h-5 w-5" />
        </div>
        <div className="flex-1 min-w-0 space-y-1.5">
          <p className="text-sm font-medium truncate">{node.attrs.fileName || "Audio"}</p>
          <audio src={node.attrs.src} controls preload="metadata" className="w-full h-8 [&::-webkit-media-controls-panel]:bg-transparent" />
        </div>
      </div>
    </NodeViewWrapper>
  );
}
