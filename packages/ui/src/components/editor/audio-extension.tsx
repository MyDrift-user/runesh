"use client";

import { Node, mergeAttributes } from "@tiptap/core";

export const AudioExtension = Node.create({
  name: "audio",
  group: "block",
  atom: true,

  addAttributes() {
    return {
      src: { default: null },
      fileName: { default: null },
    };
  },

  parseHTML() {
    return [{ tag: "div[data-audio]" }];
  },

  renderHTML({ HTMLAttributes }) {
    const { src, fileName, ...rest } = HTMLAttributes;
    return [
      "div", mergeAttributes(rest, { "data-audio": "", class: "editor-audio-wrapper" }),
      ["div", { class: "editor-audio-icon" }],
      ["div", { class: "editor-audio-content" },
        ["p", { class: "editor-audio-name" }, fileName || "Audio"],
        ["audio", { src, controls: "true", preload: "metadata", class: "editor-audio-player" }],
      ],
    ];
  },
});
