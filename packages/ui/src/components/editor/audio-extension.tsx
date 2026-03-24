"use client";

import { Node, mergeAttributes } from "@tiptap/core";

/**
 * Audio node extension for Tiptap.
 * Renders a styled `<audio>` player with filename.
 */
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
    return [{ tag: 'div[data-audio]' }];
  },

  renderHTML({ HTMLAttributes }) {
    const { src, fileName, ...rest } = HTMLAttributes;
    return [
      "div",
      mergeAttributes(rest, {
        "data-audio": "",
        class: "flex items-center gap-3 rounded-lg border bg-muted/30 p-3 my-4",
      }),
      ["div", { class: "shrink-0 text-muted-foreground" }, "🎵"],
      [
        "div",
        { class: "flex-1 min-w-0 space-y-1" },
        ["p", { class: "text-sm font-medium truncate" }, fileName || "Audio"],
        ["audio", { src, controls: "true", class: "w-full h-8", preload: "metadata" }],
      ],
    ];
  },

});
