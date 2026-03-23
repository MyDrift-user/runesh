"use client";

import { Extension } from "@tiptap/core";
import { Plugin, PluginKey } from "@tiptap/pm/state";

export interface UploadFn {
  (file: File): Promise<string>;
}

export interface FileHandlerOptions {
  /** Upload a file and return its URL */
  onUpload: UploadFn;
  /** Max file size in bytes (default: 50MB) */
  maxSize?: number;
}

/**
 * Extension that handles drag & drop and paste for all file types.
 * Routes files to the correct node type based on MIME:
 * - image/* -> image node
 * - video/* -> video node
 * - audio/* -> audio node
 * - everything else -> fileAttachment node
 */
export const FileHandlerExtension = Extension.create<FileHandlerOptions>({
  name: "fileHandler",

  addOptions() {
    return {
      onUpload: async () => "",
      maxSize: 50 * 1024 * 1024,
    };
  },

  addProseMirrorPlugins() {
    const { onUpload, maxSize } = this.options;

    const handleFiles = async (
      editor: any,
      files: File[],
      pos?: number,
    ) => {
      for (const file of files) {
        if (maxSize && file.size > maxSize) {
          console.warn(`File too large: ${file.name} (${file.size} bytes)`);
          continue;
        }

        const url = await onUpload(file);
        if (!url) continue;

        let nodeType: string;
        let attrs: Record<string, any>;

        if (file.type.startsWith("image/")) {
          nodeType = "image";
          attrs = { src: url };
        } else if (file.type.startsWith("video/")) {
          nodeType = "video";
          attrs = { src: url, fileName: file.name };
        } else if (file.type.startsWith("audio/")) {
          nodeType = "audio";
          attrs = { src: url, fileName: file.name };
        } else {
          nodeType = "fileAttachment";
          attrs = {
            src: url,
            fileName: file.name,
            fileSize: file.size,
            fileType: file.type || "application/octet-stream",
          };
        }

        const insertPos = pos ?? editor.state.selection.anchor;
        editor
          .chain()
          .focus()
          .insertContentAt(insertPos, { type: nodeType, attrs })
          .run();
      }
    };

    return [
      new Plugin({
        key: new PluginKey("fileHandler"),
        props: {
          handleDrop: (view, event) => {
            if (!event.dataTransfer?.files?.length) return false;
            event.preventDefault();

            const files = Array.from(event.dataTransfer.files);
            const pos = view.posAtCoords({
              left: event.clientX,
              top: event.clientY,
            })?.pos;

            handleFiles(this.editor, files, pos);
            return true;
          },
          handlePaste: (view, event) => {
            const files = Array.from(event.clipboardData?.files || []);
            if (!files.length) return false;

            event.preventDefault();
            handleFiles(this.editor, files);
            return true;
          },
        },
      }),
    ];
  },
});
