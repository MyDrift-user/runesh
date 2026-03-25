"use client";

import { Node, mergeAttributes } from "@tiptap/core";

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
    return [{ tag: "div[data-file-attachment]" }];
  },

  renderHTML({ HTMLAttributes }) {
    const { src, fileName, fileSize, fileType, ...rest } = HTMLAttributes;
    const sizeStr = formatSize(Number(fileSize) || 0);
    const isPdf = fileType === "application/pdf";

    if (isPdf) {
      return [
        "div", mergeAttributes(rest, { "data-file-attachment": "", class: "editor-file-pdf" }),
        ["div", { class: "editor-file-header" },
          ["span", { class: "editor-file-name" }, fileName],
          ["span", { class: "editor-file-size" }, sizeStr],
          ["a", { href: src, target: "_blank", rel: "noopener", class: "editor-file-link" }, "Open"],
        ],
        ["iframe", { src, class: "editor-file-preview", title: fileName }],
      ];
    }

    return [
      "div", mergeAttributes(rest, { "data-file-attachment": "", class: "editor-file-wrapper" }),
      ["div", { class: "editor-file-icon" }],
      ["div", { class: "editor-file-info" },
        ["span", { class: "editor-file-name" }, fileName || "File"],
        ["span", { class: "editor-file-size" }, sizeStr],
      ],
      ["a", { href: src, download: fileName, class: "editor-file-download", title: "Download" }],
    ];
  },
});

function formatSize(bytes: number): string {
  if (bytes <= 0) return "";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const val = bytes / 1024 ** i;
  return `${val < 10 ? val.toFixed(1) : Math.round(val)} ${units[i]}`;
}
