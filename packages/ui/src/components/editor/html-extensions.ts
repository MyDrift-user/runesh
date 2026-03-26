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

const DeletedMark = Mark.create({
  name: "deleted",
  parseHTML() {
    return [{ tag: "del" }, { tag: "s" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["del", mergeAttributes(HTMLAttributes), 0];
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

const VarMark = Mark.create({
  name: "var",
  parseHTML() {
    return [{ tag: "var" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["var", mergeAttributes(HTMLAttributes), 0];
  },
});

// ── Block nodes ──────────────────────────────────────────────────────────────

const Details = Node.create({
  name: "details",
  group: "block",
  content: "detailsSummary block+",
  defining: true,
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
  defining: true,
  parseHTML() {
    return [{ tag: "summary" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["summary", mergeAttributes(HTMLAttributes), 0];
  },
});

const Figure = Node.create({
  name: "figure",
  group: "block",
  content: "(block | image) figcaption?",
  parseHTML() {
    return [{ tag: "figure" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["figure", mergeAttributes(HTMLAttributes), 0];
  },
});

const Figcaption = Node.create({
  name: "figcaption",
  content: "inline*",
  parseHTML() {
    return [{ tag: "figcaption" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["figcaption", mergeAttributes(HTMLAttributes), 0];
  },
});

const DefinitionList = Node.create({
  name: "definitionList",
  group: "block",
  content: "(definitionTerm | definitionDescription)+",
  parseHTML() {
    return [{ tag: "dl" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["dl", mergeAttributes(HTMLAttributes), 0];
  },
});

const DefinitionTerm = Node.create({
  name: "definitionTerm",
  content: "inline*",
  defining: true,
  parseHTML() {
    return [{ tag: "dt" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["dt", mergeAttributes(HTMLAttributes), 0];
  },
});

const DefinitionDescription = Node.create({
  name: "definitionDescription",
  content: "block+",
  defining: true,
  parseHTML() {
    return [{ tag: "dd" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["dd", mergeAttributes(HTMLAttributes), 0];
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
  DeletedMark,
  SampMark,
  VarMark,
  Details,
  DetailsSummary,
  Figure,
  Figcaption,
  DefinitionList,
  DefinitionTerm,
  DefinitionDescription,
];
