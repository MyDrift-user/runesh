"use client";

import { Node, mergeAttributes } from "@tiptap/core";

/**
 * Video node extension for Tiptap.
 * Renders an inline `<video>` player with controls.
 */
export const VideoExtension = Node.create({
  name: "video",
  group: "block",
  atom: true,

  addAttributes() {
    return {
      src: { default: null },
      fileName: { default: null },
    };
  },

  parseHTML() {
    return [{ tag: "video[src]" }];
  },

  renderHTML({ HTMLAttributes }) {
    return [
      "div",
      { class: "video-wrapper my-4" },
      [
        "video",
        mergeAttributes(HTMLAttributes, {
          controls: "true",
          class: "w-full rounded-lg",
          preload: "metadata",
        }),
      ],
    ];
  },

});
