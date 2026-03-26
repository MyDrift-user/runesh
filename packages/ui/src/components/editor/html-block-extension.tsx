"use client";

import { Node, mergeAttributes } from "@tiptap/core";

export const HtmlBlockExtension = Node.create({
  name: "htmlBlock",
  group: "block",
  atom: true,

  addAttributes() {
    return {
      html: { default: "" },
    };
  },

  parseHTML() {
    return [{ tag: "div[data-html-block]" }];
  },

  renderHTML({ HTMLAttributes }) {
    return [
      "div",
      mergeAttributes(HTMLAttributes, { "data-html-block": "" }),
    ];
  },

  addNodeView() {
    return ({ node, editor, getPos }) => {
      const wrapper = document.createElement("div");
      wrapper.className = "editor-html-block";
      wrapper.setAttribute("data-html-block", "");

      let editing = !node.attrs.html;

      // Preview area
      const preview = document.createElement("div");
      preview.className = "editor-html-preview";

      // Code area
      const codeWrap = document.createElement("div");
      codeWrap.className = "editor-html-code";
      const textarea = document.createElement("textarea");
      textarea.className = "editor-html-textarea";
      textarea.value = node.attrs.html || "";
      textarea.placeholder = "Paste or type HTML here...";
      textarea.spellcheck = false;
      codeWrap.appendChild(textarea);

      // Toolbar
      const toolbar = document.createElement("div");
      toolbar.className = "editor-html-toolbar";

      const toggleBtn = document.createElement("button");
      toggleBtn.className = "editor-html-toggle";
      const label = document.createElement("span");
      label.className = "editor-html-label";
      toolbar.appendChild(toggleBtn);
      toolbar.appendChild(label);

      const render = () => {
        const html = node.attrs.html || "";
        if (editing) {
          preview.style.display = "none";
          codeWrap.style.display = "block";
          textarea.value = html;
          toggleBtn.textContent = "Preview";
          label.textContent = "HTML";
        } else {
          codeWrap.style.display = "none";
          if (html) {
            preview.style.display = "block";
            preview.innerHTML = html;
          } else {
            preview.style.display = "none";
          }
          toggleBtn.textContent = "Edit";
          label.textContent = "HTML";
        }
      };

      toggleBtn.addEventListener("mousedown", (e) => {
        e.preventDefault();
        e.stopPropagation();
        if (editing) {
          // Save and switch to preview
          const pos = typeof getPos === "function" ? getPos() : null;
          if (pos != null) {
            editor.chain().focus()
              .command(({ tr }) => {
                tr.setNodeMarkup(pos, undefined, { html: textarea.value });
                return true;
              }).run();
          }
        }
        editing = !editing;
        render();
      });

      // Auto-resize textarea
      textarea.addEventListener("input", () => {
        textarea.style.height = "auto";
        textarea.style.height = textarea.scrollHeight + "px";
      });

      wrapper.appendChild(toolbar);
      wrapper.appendChild(codeWrap);
      wrapper.appendChild(preview);
      render();

      // Auto-resize on first render
      requestAnimationFrame(() => {
        if (editing) {
          textarea.style.height = "auto";
          textarea.style.height = textarea.scrollHeight + "px";
        }
      });

      return {
        dom: wrapper,
        stopEvent(event) {
          const target = event.target as HTMLElement;
          return !!target.closest("textarea, button, .editor-html-preview");
        },
        ignoreMutation() {
          return true;
        },
        update(updatedNode) {
          if (updatedNode.type.name !== "htmlBlock") return false;
          if (!editing) {
            const html = updatedNode.attrs.html || "";
            if (html) {
              preview.innerHTML = html;
              preview.style.display = "block";
            } else {
              preview.style.display = "none";
            }
          } else {
            textarea.value = updatedNode.attrs.html || "";
          }
          return true;
        },
      };
    };
  },
});
