"use client";

import { Node, Mark, Extension, mergeAttributes } from "@tiptap/core";
import { Plugin, PluginKey } from "@tiptap/pm/state";

// ── Inline marks for common HTML elements ────────────────────────────────────

const Kbd = Mark.create({
  name: "kbd",
  parseHTML() { return [{ tag: "kbd" }]; },
  renderHTML({ HTMLAttributes }) { return ["kbd", mergeAttributes(HTMLAttributes), 0]; },
});

const Superscript = Mark.create({
  name: "superscript",
  parseHTML() { return [{ tag: "sup" }]; },
  renderHTML({ HTMLAttributes }) { return ["sup", mergeAttributes(HTMLAttributes), 0]; },
});

const Subscript = Mark.create({
  name: "subscript",
  parseHTML() { return [{ tag: "sub" }]; },
  renderHTML({ HTMLAttributes }) { return ["sub", mergeAttributes(HTMLAttributes), 0]; },
});

const SmallMark = Mark.create({
  name: "small",
  parseHTML() { return [{ tag: "small" }]; },
  renderHTML({ HTMLAttributes }) { return ["small", mergeAttributes(HTMLAttributes), 0]; },
});

const InsertedMark = Mark.create({
  name: "inserted",
  parseHTML() { return [{ tag: "ins" }]; },
  renderHTML({ HTMLAttributes }) { return ["ins", mergeAttributes(HTMLAttributes), 0]; },
});

// ── Raw HTML block — renders arbitrary HTML as a non-editable block ──────────

const RawHtmlBlock = Node.create({
  name: "rawHtml",
  group: "block",
  atom: true,
  draggable: false,

  addAttributes() {
    return {
      content: {
        default: "",
        parseHTML: (el: HTMLElement) => el.getAttribute("data-html-content") || "",
        renderHTML: (attrs: Record<string, string>) => ({ "data-html-content": attrs.content }),
      },
    };
  },

  parseHTML() {
    return [{ tag: "div[data-html-content]" }];
  },

  renderHTML({ HTMLAttributes }) {
    return ["div", mergeAttributes(HTMLAttributes, { "data-raw-html": "" })];
  },

  addNodeView() {
    return ({ node }) => {
      const wrapper = document.createElement("div");
      wrapper.className = "editor-raw-html";
      wrapper.setAttribute("data-raw-html", "");
      wrapper.innerHTML = node.attrs.content || "";

      return {
        dom: wrapper,
        stopEvent() { return true; },
        ignoreMutation() { return true; },
        update(updatedNode) {
          if (updatedNode.type.name !== "rawHtml") return false;
          wrapper.innerHTML = updatedNode.attrs.content || "";
          return true;
        },
      };
    };
  },
});

// ── HTML preprocessor — converts complex HTML to rawHtml blocks before parse ─

// Tags that ProseMirror/Tiptap already handles natively
const NATIVE_BLOCK_TAGS = new Set([
  "p", "h1", "h2", "h3", "h4", "h5", "h6",
  "ul", "ol", "li", "blockquote", "pre",
  "hr", "br", "img",
]);

function isComplexHtml(el: Element): boolean {
  const tag = el.tagName.toLowerCase();

  // Native tags without special attributes → let ProseMirror handle
  if (NATIVE_BLOCK_TAGS.has(tag) && !el.getAttribute("align") && !el.getAttribute("style")) {
    return false;
  }

  // Divs/sections with attributes (align, style, class) → complex
  if ((tag === "div" || tag === "section" || tag === "article" || tag === "aside" || tag === "nav" || tag === "header" || tag === "footer" || tag === "main") &&
      (el.attributes.length > 0)) {
    return true;
  }

  // Tables → already handled by tiptap table extension for simple cases,
  // but layout tables (with style/align/rowspan) should be raw
  if (tag === "table" && (el.querySelector("[style]") || el.querySelector("[align]") || el.querySelector("[rowspan]") || el.querySelector("[colspan]"))) {
    return true;
  }

  // Any element with significant attributes that ProseMirror would strip
  if (el.getAttribute("align") || el.getAttribute("style")) {
    return true;
  }

  // Unknown/custom elements
  if (!NATIVE_BLOCK_TAGS.has(tag) && !["div", "span", "table", "thead", "tbody", "tfoot", "tr", "td", "th", "a", "strong", "em", "code", "b", "i", "u", "s", "del", "sup", "sub", "small", "kbd", "ins", "samp", "abbr", "details", "summary"].includes(tag)) {
    return true;
  }

  return false;
}

function preprocessHtml(html: string): string {
  // Quick check: if no HTML tags at all, return as-is
  if (!html.includes("<")) return html;

  const template = document.createElement("template");
  template.innerHTML = html;
  const frag = template.content;

  const children = Array.from(frag.children);
  let modified = false;

  for (const child of children) {
    if (isComplexHtml(child)) {
      const wrapper = document.createElement("div");
      wrapper.setAttribute("data-html-content", child.outerHTML);
      child.replaceWith(wrapper);
      modified = true;
    }
  }

  if (!modified) return html;

  const div = document.createElement("div");
  div.appendChild(frag);
  return div.innerHTML;
}

const HtmlPreprocessor = Extension.create({
  name: "htmlPreprocessor",

  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: new PluginKey("htmlPreprocessor"),
        props: {
          transformPastedHTML(html: string) {
            return preprocessHtml(html);
          },
        },
      }),
    ];
  },
});

// ── Export ────────────────────────────────────────────────────────────────────

export const htmlExtensions = [
  Kbd,
  Superscript,
  Subscript,
  SmallMark,
  InsertedMark,
  RawHtmlBlock,
  HtmlPreprocessor,
];
