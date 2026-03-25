"use client";

import { Node, mergeAttributes } from "@tiptap/core";
import { ReactNodeViewRenderer } from "@tiptap/react";
import { VideoView } from "./video-view";

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
    return [{ tag: "div[data-video]" }];
  },

  renderHTML({ HTMLAttributes }) {
    return ["div", mergeAttributes({ "data-video": "" }, HTMLAttributes)];
  },

  addNodeView() {
    return ReactNodeViewRenderer(VideoView);
  },
});
