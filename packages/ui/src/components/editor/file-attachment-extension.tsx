"use client";

import { Node, mergeAttributes } from "@tiptap/core";

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
  "text/plain": "📃",
  "text/csv": "📊",
};

function getFileIcon(mimeType: string): string {
  return FILE_ICONS[mimeType] || "📎";
}

function formatSize(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const val = bytes / 1024 ** i;
  return `${val < 10 ? val.toFixed(1) : Math.round(val)} ${units[i]}`;
}

/**
 * Generic file attachment node for Tiptap.
 * Renders a card with icon, filename, size, and download link.
 * For PDFs, shows an embedded preview.
 */
export const FileAttachmentExtension = Node.create({
  name: "fileAttachment",
  group: "block",
  atom: true,

  addAttributes() {
    return {
      src: { default: null },
      fileName: { default: "File" },
      fileSize: { default: 0 },
      fileType: { default: "application/octet-stream" },
    };
  },

  parseHTML() {
    return [{ tag: 'div[data-file-attachment]' }];
  },

  renderHTML({ HTMLAttributes }) {
    const { src, fileName, fileSize, fileType, ...rest } = HTMLAttributes;
    const icon = getFileIcon(fileType);
    const size = formatSize(Number(fileSize) || 0);
    const isPdf = fileType === "application/pdf";

    if (isPdf) {
      return [
        "div",
        mergeAttributes(rest, {
          "data-file-attachment": "",
          class: "my-4 space-y-2",
        }),
        [
          "div",
          { class: "flex items-center gap-2 text-sm" },
          ["span", {}, icon],
          ["span", { class: "font-medium" }, fileName],
          ["span", { class: "text-muted-foreground" }, `(${size})`],
          ["a", { href: src, target: "_blank", rel: "noopener", class: "ml-auto text-primary hover:underline text-xs" }, "Open"],
        ],
        [
          "iframe",
          { src, class: "w-full h-96 rounded-lg border", title: fileName },
        ],
      ];
    }

    return [
      "div",
      mergeAttributes(rest, {
        "data-file-attachment": "",
        class: "flex items-center gap-3 rounded-lg border bg-muted/30 p-3 my-4 not-prose",
      }),
      ["span", { class: "text-2xl shrink-0" }, icon],
      [
        "div",
        { class: "flex-1 min-w-0" },
        ["p", { class: "text-sm font-medium truncate" }, fileName],
        ["p", { class: "text-xs text-muted-foreground" }, size],
      ],
      [
        "a",
        {
          href: src,
          download: fileName,
          class: "shrink-0 text-xs text-primary hover:underline",
        },
        "Download",
      ],
    ];
  },

});
