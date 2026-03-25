"use client";

import { NodeViewWrapper } from "@tiptap/react";
import { Loader2 } from "lucide-react";

/**
 * Upload placeholder shown while a file is being uploaded.
 * Displays a skeleton with a spinner and filename.
 */
export function UploadPlaceholder({ node }: { node: any }) {
  const { fileName, fileType, progress } = node.attrs;

  const isImage = fileType?.startsWith("image/");
  const isVideo = fileType?.startsWith("video/");
  const isAudio = fileType?.startsWith("audio/");

  const label = isImage ? "Uploading image..." : isVideo ? "Uploading video..." : isAudio ? "Uploading audio..." : "Uploading file...";

  return (
    <NodeViewWrapper className="my-4">
      <div className="flex items-center gap-3 rounded-lg border border-dashed border-border bg-muted/30 p-4 animate-pulse">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground shrink-0" />
        <div className="flex-1 min-w-0">
          <p className="text-sm font-medium text-foreground truncate">{fileName || label}</p>
          {progress !== undefined && progress >= 0 && (
            <div className="mt-1.5 h-1.5 w-full rounded-full bg-muted overflow-hidden">
              <div
                className="h-full rounded-full bg-primary transition-all duration-300"
                style={{ width: `${Math.min(progress, 100)}%` }}
              />
            </div>
          )}
        </div>
      </div>
    </NodeViewWrapper>
  );
}
