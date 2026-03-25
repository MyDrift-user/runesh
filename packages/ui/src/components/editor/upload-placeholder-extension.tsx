"use client";

import { Node, mergeAttributes } from "@tiptap/core";
import { ReactNodeViewRenderer } from "@tiptap/react";
import { UploadPlaceholder } from "./upload-placeholder";

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
    return ["div", mergeAttributes({ "data-upload-placeholder": "" }, HTMLAttributes)];
  },

  addNodeView() {
    return ReactNodeViewRenderer(UploadPlaceholder);
  },
});
