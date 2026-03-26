"use client";

import { Node, Mark, mergeAttributes } from "@tiptap/core";

// ── Inline marks ─────────────────────────────────────────────────────────────

const Kbd = Mark.create({
  name: "kbd",
  parseHTML() {
    return [{ tag: "kbd" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["kbd", mergeAttributes(HTMLAttributes), 0];
  },
});

const Superscript = Mark.create({
  name: "superscript",
  parseHTML() {
    return [{ tag: "sup" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["sup", mergeAttributes(HTMLAttributes), 0];
  },
});

const Subscript = Mark.create({
  name: "subscript",
  parseHTML() {
    return [{ tag: "sub" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["sub", mergeAttributes(HTMLAttributes), 0];
  },
});

const SmallMark = Mark.create({
  name: "small",
  parseHTML() {
    return [{ tag: "small" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["small", mergeAttributes(HTMLAttributes), 0];
  },
});

const Abbreviation = Mark.create({
  name: "abbreviation",
  addAttributes() {
    return { title: { default: null } };
  },
  parseHTML() {
    return [{ tag: "abbr" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["abbr", mergeAttributes(HTMLAttributes), 0];
  },
});

const InsertedMark = Mark.create({
  name: "inserted",
  parseHTML() {
    return [{ tag: "ins" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["ins", mergeAttributes(HTMLAttributes), 0];
  },
});

const SampMark = Mark.create({
  name: "samp",
  parseHTML() {
    return [{ tag: "samp" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["samp", mergeAttributes(HTMLAttributes), 0];
  },
});

// ── Block nodes ──────────────────────────────────────────────────────────────

const Details = Node.create({
  name: "details",
  group: "block",
  content: "detailsSummary block*",
  parseHTML() {
    return [{ tag: "details" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["details", mergeAttributes(HTMLAttributes), 0];
  },
});

const DetailsSummary = Node.create({
  name: "detailsSummary",
  content: "inline*",
  parseHTML() {
    return [{ tag: "summary" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["summary", mergeAttributes(HTMLAttributes), 0];
  },
});

// ── Catch-all: preserves unknown block-level HTML as non-editable ────────────

const RawHtmlBlock = Node.create({
  name: "rawHtmlBlock",
  group: "block",
  atom: true,
  draggable: false,

  addAttributes() {
    return {
      html: { default: "" },
      tagName: { default: "div" },
    };
  },

  parseHTML() {
    return [{
      tag: "*",
      priority: 0,
      getAttrs(dom: HTMLElement) {
        // Skip elements that other extensions handle
        const tag = dom.tagName.toLowerCase();
        const skip = new Set([
          "p", "h1", "h2", "h3", "h4", "h5", "h6",
          "ul", "ol", "li", "blockquote", "pre", "code",
          "table", "thead", "tbody", "tfoot", "tr", "td", "th",
          "hr", "br", "img", "a", "video", "audio", "iframe",
          "details", "summary", "figure", "figcaption",
          "div", "span", "input", "label", "form", "button",
        ]);
        if (skip.has(tag)) return false;
        return { html: dom.outerHTML, tagName: tag };
      },
    }];
  },

  renderHTML({ HTMLAttributes }) {
    return ["div", { "data-raw-html": "", class: "editor-raw-html" }];
  },

  addNodeView() {
    return ({ node }) => {
      const wrapper = document.createElement("div");
      wrapper.className = "editor-raw-html";
      wrapper.setAttribute("data-raw-html", "");
      wrapper.innerHTML = node.attrs.html || "";

      return {
        dom: wrapper,
        stopEvent() { return true; },
        ignoreMutation() { return true; },
        update(updatedNode) {
          if (updatedNode.type.name !== "rawHtmlBlock") return false;
          wrapper.innerHTML = updatedNode.attrs.html || "";
          return true;
        },
      };
    };
  },
});

// ── Export ────────────────────────────────────────────────────────────────────

export const htmlExtensions = [
  Kbd,
  Superscript,
  Subscript,
  SmallMark,
  Abbreviation,
  InsertedMark,
  SampMark,
  Details,
  DetailsSummary,
  RawHtmlBlock,
];
