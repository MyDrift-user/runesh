"use client";

import { Node, mergeAttributes } from "@tiptap/core";
import { ReactNodeViewRenderer } from "@tiptap/react";
import { AudioView } from "./audio-view";

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
    return ["div", mergeAttributes({ "data-audio": "" }, HTMLAttributes)];
  },

  addNodeView() {
    return ReactNodeViewRenderer(AudioView);
  },
});
