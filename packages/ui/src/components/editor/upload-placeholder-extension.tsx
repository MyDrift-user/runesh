"use client";

import { Node, mergeAttributes } from "@tiptap/core";

export const UploadPlaceholderExtension = Node.create({
  name: "uploadPlaceholder",
  group: "block",
  atom: true,

  addAttributes() {
    return {
      id: { default: null },
      fileName: { default: null },
      fileType: { default: null },
      progress: { default: 0 },
    };
  },

  parseHTML() {
    return [{ tag: "div[data-upload-placeholder]" }];
  },

  renderHTML({ HTMLAttributes }) {
    const { fileName, fileType } = HTMLAttributes;
    return [
      "div",
      mergeAttributes({ "data-upload-placeholder": "", class: "editor-upload-placeholder" }),
      ["div", { class: "editor-upload-spinner" }],
      ["span", { class: "editor-upload-text" }, fileName || "Uploading..."],
    ];
  },
});
