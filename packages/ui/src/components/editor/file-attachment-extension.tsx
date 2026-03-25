"use client";

import { Node, mergeAttributes } from "@tiptap/core";
import { ReactNodeViewRenderer } from "@tiptap/react";
import { FileView } from "./file-view";

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
    return ["div", mergeAttributes({ "data-file-attachment": "" }, HTMLAttributes)];
  },

  addNodeView() {
    return ReactNodeViewRenderer(FileView);
  },
});
