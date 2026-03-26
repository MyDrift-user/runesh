"use client";

import { Node, mergeAttributes } from "@tiptap/core";

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
    const { src, fileName, ...rest } = HTMLAttributes;
    return [
      "div", mergeAttributes(rest, { "data-video": "", class: "editor-video-wrapper" }),
      ["video", { src, controls: "true", preload: "metadata", class: "editor-video" }],
      ...(fileName ? [["div", { class: "editor-video-footer" }, fileName]] : []),
    ];
  },

  addNodeView() {
    return ({ node }) => {
      const wrapper = document.createElement("div");
      wrapper.setAttribute("data-video", "");
      wrapper.className = "editor-video-wrapper";

      const video = document.createElement("video");
      video.src = node.attrs.src || "";
      video.controls = true;
      video.preload = "metadata";
      video.className = "editor-video";
      wrapper.appendChild(video);

      if (node.attrs.fileName) {
        const footer = document.createElement("div");
        footer.className = "editor-video-footer";
        footer.textContent = node.attrs.fileName;
        wrapper.appendChild(footer);
      }

      return {
        dom: wrapper,
        stopEvent(event) {
          const target = event.target as HTMLElement;
          return !!target.closest("video");
        },
        update(updatedNode) {
          if (updatedNode.type.name !== "video") return false;
          video.src = updatedNode.attrs.src || "";
          return true;
        },
      };
    };
  },
});
