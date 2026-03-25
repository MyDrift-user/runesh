"use client";

import { Extension } from "@tiptap/core";
import { Plugin, PluginKey } from "@tiptap/pm/state";

export interface UploadFn {
  (file: File): Promise<string>;
}

export interface FileHandlerOptions {
  onUpload: UploadFn;
  maxSize?: number;
}

/**
 * Extension that handles drag & drop and paste for all file types.
 * Inserts a placeholder with loading indicator during upload,
 * then replaces it with the actual media node when done.
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

    const handleFiles = async (editor: any, files: File[], pos?: number) => {
      for (const file of files) {
        if (maxSize && file.size > maxSize) continue;

        const uploadId = Math.random().toString(36).slice(2);
        const insertPos = pos ?? editor.state.selection.anchor;

        // Insert placeholder
        editor.chain().focus().insertContentAt(insertPos, {
          type: "uploadPlaceholder",
          attrs: { id: uploadId, fileName: file.name, fileType: file.type, progress: 0 },
        }).run();

        // Upload
        try {
          const url = await onUpload(file);
          if (!url) {
            removePlaceholder(editor, uploadId);
            continue;
          }

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
            attrs = { src: url, fileName: file.name, fileSize: file.size, fileType: file.type };
          }

          // Replace placeholder with actual node
          replacePlaceholder(editor, uploadId, nodeType, attrs);
        } catch {
          removePlaceholder(editor, uploadId);
        }
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
            const pos = view.posAtCoords({ left: event.clientX, top: event.clientY })?.pos;
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

function removePlaceholder(editor: any, id: string) {
  const { doc } = editor.state;
  doc.descendants((node: any, pos: number) => {
    if (node.type.name === "uploadPlaceholder" && node.attrs.id === id) {
      editor.chain().focus().deleteRange({ from: pos, to: pos + node.nodeSize }).run();
      return false;
    }
  });
}

function replacePlaceholder(editor: any, id: string, nodeType: string, attrs: Record<string, any>) {
  const { doc } = editor.state;
  doc.descendants((node: any, pos: number) => {
    if (node.type.name === "uploadPlaceholder" && node.attrs.id === id) {
      editor.chain()
        .focus()
        .deleteRange({ from: pos, to: pos + node.nodeSize })
        .insertContentAt(pos, { type: nodeType, attrs })
        .run();
      return false;
    }
  });
}
