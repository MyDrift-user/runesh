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

// Inline span with style/class support
const SpanMark = Mark.create({
  name: "span",
  parseHTML() {
    return [{ tag: "span" }];
  },
  addAttributes() {
    return {
      style: { default: null },
      class: { default: null },
    };
  },
  renderHTML({ HTMLAttributes }) {
    return ["span", mergeAttributes(HTMLAttributes), 0];
  },
});

// ── Block nodes ──────────────────────────────────────────────────────────────

// Div block — preserves align, class, style attributes
const DivBlock = Node.create({
  name: "div",
  group: "block",
  content: "block*",
  parseHTML() {
    return [{ tag: "div" }];
  },
  addAttributes() {
    return {
      align: { default: null },
      style: { default: null },
      class: { default: null },
    };
  },
  renderHTML({ HTMLAttributes }) {
    return ["div", mergeAttributes(HTMLAttributes), 0];
  },
});

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

// ── Export ────────────────────────────────────────────────────────────────────

export const htmlExtensions = [
  Kbd,
  Superscript,
  Subscript,
  SmallMark,
  Abbreviation,
  InsertedMark,
  SampMark,
  SpanMark,
  DivBlock,
  Details,
  DetailsSummary,
];
