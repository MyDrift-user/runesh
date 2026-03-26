"use client";

import { Node, mergeAttributes } from "@tiptap/core";

export const VideoExtension = Node.create({
  name: "video",
  group: "block",
  atom: true,
  draggable: false,

  addAttributes() {
    return {
      src: { default: null },
      fileName: { default: null },
    };
  },

  parseHTML() {
    return [{ tag: "div[data-video]" }];
  },

  renderHTML({ HTMLAttributes }) {
    const { src, fileName, ...rest } = HTMLAttributes;
    return [
      "div", mergeAttributes(rest, { "data-video": "", class: "editor-video-wrapper", draggable: "false" }),
      ["video", { src, controls: "true", preload: "metadata", class: "editor-video" }],
      ...(fileName ? [["div", { class: "editor-video-footer" }, fileName]] : []),
    ];
  },
});
