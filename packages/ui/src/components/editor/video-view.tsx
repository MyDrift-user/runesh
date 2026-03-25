"use client";

import { NodeViewWrapper } from "@tiptap/react";

export function VideoView({ node, selected }: { node: any; selected: boolean }) {
  return (
    <NodeViewWrapper className="my-4">
      <div className={`rounded-lg overflow-hidden border border-border ${selected ? "ring-2 ring-primary" : ""}`}>
        <video
          src={node.attrs.src}
          controls
          preload="metadata"
          className="w-full"
        />
        {node.attrs.fileName && (
          <div className="px-3 py-1.5 bg-muted/30 text-xs text-muted-foreground truncate border-t border-border">
            {node.attrs.fileName}
          </div>
        )}
      </div>
    </NodeViewWrapper>
  );
}
