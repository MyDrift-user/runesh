"use client";

import { Mark, mergeAttributes } from "@tiptap/core";

// Additional inline marks not covered by StarterKit or tiptap-markdown

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

export const htmlExtensions = [
  Kbd,
  Superscript,
  Subscript,
  SmallMark,
  InsertedMark,
];
