"use client";

import { Node, mergeAttributes } from "@tiptap/core";

export const AudioExtension = Node.create({
  name: "audio",
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

  addNodeView() {
    return ({ node }) => {
      const wrapper = document.createElement("div");
      wrapper.setAttribute("data-audio", "");
      wrapper.className = "editor-audio-wrapper";

      const icon = document.createElement("div");
      icon.className = "editor-audio-icon";
      wrapper.appendChild(icon);

      const content = document.createElement("div");
      content.className = "editor-audio-content";

      const name = document.createElement("p");
      name.className = "editor-audio-name";
      name.textContent = node.attrs.fileName || "Audio";
      content.appendChild(name);

      const audio = document.createElement("audio");
      audio.src = node.attrs.src || "";
      audio.controls = true;
      audio.preload = "metadata";
      audio.className = "editor-audio-player";

      audio.addEventListener("dragstart", (e) => e.preventDefault());

      content.appendChild(audio);
      wrapper.appendChild(content);

      return {
        dom: wrapper,
        stopEvent() {
          return true;
        },
        ignoreMutation() {
          return true;
        },
        update(updatedNode) {
          if (updatedNode.type.name !== "audio") return false;
          audio.src = updatedNode.attrs.src || "";
          name.textContent = updatedNode.attrs.fileName || "Audio";
          return true;
        },
      };
    };
  },
});
