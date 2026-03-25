"use client";

import { NodeViewWrapper } from "@tiptap/react";

export function ImageView({ node, selected }: { node: any; selected: boolean }) {
  return (
    <NodeViewWrapper className="my-4">
      <img
        src={node.attrs.src}
        alt={node.attrs.alt || ""}
        className={`rounded-lg max-w-full h-auto transition-shadow ${selected ? "ring-2 ring-primary shadow-lg" : ""}`}
      />
    </NodeViewWrapper>
  );
}
