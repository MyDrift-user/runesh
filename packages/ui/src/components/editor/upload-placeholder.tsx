"use client";

import { NodeViewWrapper } from "@tiptap/react";
import { Loader2, ImageIcon, Film, Volume2, FileUp } from "lucide-react";

export function UploadPlaceholder({ node }: { node: any }) {
  const { fileName, fileType } = node.attrs;

  const isImage = fileType?.startsWith("image/");
  const isVideo = fileType?.startsWith("video/");
  const isAudio = fileType?.startsWith("audio/");

  const Icon = isImage ? ImageIcon : isVideo ? Film : isAudio ? Volume2 : FileUp;
  const label = isImage ? "Uploading image..." : isVideo ? "Uploading video..." : isAudio ? "Uploading audio..." : "Uploading file...";

  return (
    <NodeViewWrapper className="my-3 not-prose">
      <div className="flex items-center gap-3 rounded-lg border border-dashed border-primary/30 bg-primary/5 px-4 py-3">
        <div className="relative flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-primary/10">
          <Icon className="h-4 w-4 text-primary/60" />
          <Loader2 className="absolute h-9 w-9 animate-spin text-primary/30" />
        </div>
        <div className="flex-1 min-w-0">
          <p className="text-sm font-medium truncate leading-none">{fileName || label}</p>
          <p className="text-xs text-muted-foreground mt-0.5">{label}</p>
        </div>
      </div>
    </NodeViewWrapper>
  );
}
