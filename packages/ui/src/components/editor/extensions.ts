import {
    CodeBlockLowlight,
    HorizontalRule,
    StarterKit,
    TaskItem,
    TaskList,
    TiptapLink,
    TiptapUnderline,
    CustomKeymap,
    HighlightExtension,
    TextStyle,
    Color,
} from "novel";
import { Node, Extension, type Extensions } from "@tiptap/core";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { Table } from "@tiptap/extension-table";
import { TableRow } from "@tiptap/extension-table-row";
import { TableCell } from "@tiptap/extension-table-cell";
import { TableHeader } from "@tiptap/extension-table-header";
import GlobalDragHandle from "tiptap-extension-global-drag-handle";
import Image from "@tiptap/extension-image";
import { cx } from "class-variance-authority";
import { VideoExtension } from "./video-extension";
import { AudioExtension } from "./audio-extension";
import { FileAttachmentExtension } from "./file-attachment-extension";
import { UploadPlaceholderExtension } from "./upload-placeholder-extension";
import { htmlExtensions } from "./html-extensions";
import { Markdown } from "tiptap-markdown";
import { common, createLowlight } from "lowlight";

/**
 * FixDragHandleDrop - When a table is drag-moved via the global drag handle,
 * ProseMirror's default drop handler fails to delete the source table because:
 *  - The drag handle sets `view.dragging` as a plain object without the `node`
 *    field, so PM falls back to `tr.deleteSelection()`.
 *  - The browser's native drag gestures cause DOM mutations that PM's observer
 *    picks up, shifting the selection away from the original NodeSelection on
 *    the table -- so `deleteSelection` is a no-op.
 *  - The browser's native contentEditable drag clears the *text* from the
 *    source cells but can't remove the table element structure, leaving an
 *    empty ghost table.
 *
 * Fix: after any drop transaction, scan for tables whose cells all contain only
 * empty paragraphs. If such tables appeared as a result of the drop (i.e. they
 * had text content before), delete them via `appendTransaction`.
 */
const FixDragHandleDrop = Extension.create({
    name: "fixDragHandleDrop",
    addProseMirrorPlugins() {
        return [
            new Plugin({
                key: new PluginKey("fixDragHandleDrop"),
                appendTransaction(transactions, oldState, newState) {
                    const isDrop = transactions.some(
                        (tr) => tr.getMeta("uiEvent") === "drop",
                    );
                    if (!isDrop) return null;

                    const newEmptyPositions: Array<{ from: number; to: number }> = [];
                    newState.doc.descendants(
                        (node: { type: { name: string }; nodeSize: number; descendants: (cb: (n: any) => boolean | void) => void }, pos: number) => {
                            if (node.type.name !== "table") return;
                            if (!tableIsEmpty(node)) return;
                            newEmptyPositions.push({ from: pos, to: pos + node.nodeSize });
                        },
                    );
                    if (newEmptyPositions.length === 0) return null;

                    const oldEmptySet = new Set<string>();
                    oldState.doc.descendants(
                        (node: { type: { name: string }; nodeSize: number; textContent: string; descendants: (cb: (n: any) => boolean | void) => void }, pos: number) => {
                            if (node.type.name !== "table") return;
                            if (tableIsEmpty(node)) {
                                oldEmptySet.add(`${node.type.name}:${node.nodeSize}`);
                            }
                        },
                    );

                    const toDelete = newEmptyPositions.filter(
                        (t) => {
                            const node = newState.doc.nodeAt(t.from);
                            if (!node) return false;
                            const fp = `${node.type.name}:${node.nodeSize}`;
                            if (oldEmptySet.has(fp)) {
                                oldEmptySet.delete(fp);
                                return false;
                            }
                            return true;
                        },
                    );
                    if (toDelete.length === 0) return null;

                    const tr = newState.tr;
                    for (let i = toDelete.length - 1; i >= 0; i--) {
                        tr.delete(toDelete[i].from, toDelete[i].to);
                    }
                    return tr;
                },
            }),
        ];
    },
});

function tableIsEmpty(table: { descendants: (cb: (node: { isText: boolean }) => boolean | void) => void }): boolean {
    let hasText = false;
    table.descendants((node) => {
        if (node.isText) {
            hasText = true;
            return false;
        }
    });
    return !hasText;
}

/**
 * TrailingNode - guarantees there is always an empty paragraph at the
 * bottom of the document so the user can always click / arrow-key past
 * the last block (code block, image, hr, etc.) and keep typing.
 */
const TrailingNode = Node.create({
    name: "trailingNode",
    addProseMirrorPlugins() {
        const pluginKey = new PluginKey("trailingNode");

        return [
            new Plugin({
                key: pluginKey,
                appendTransaction(_transactions, _oldState, newState) {
                    const { doc, tr, schema } = newState as any;
                    const lastNode = doc.lastChild;
                    const shouldInsert = lastNode && lastNode.type.name !== "paragraph";
                    if (shouldInsert) {
                        return tr.insert(doc.content.size, schema.nodes.paragraph.create());
                    }
                    return undefined;
                },
            }),
        ];
    },
});

const tiptapLink = TiptapLink.configure({
    HTMLAttributes: {
        class: cx(
            "text-primary underline underline-offset-[3px] hover:text-primary/80 transition-colors cursor-pointer"
        ),
    },
});

const taskList = TaskList.configure({
    HTMLAttributes: {
        class: cx("not-prose pl-2"),
    },
});

const taskItem = TaskItem.configure({
    HTMLAttributes: {
        class: cx("flex gap-2 items-start my-4"),
    },
    nested: true,
});

const horizontalRule = HorizontalRule.configure({
    HTMLAttributes: {
        class: cx("mt-4 mb-6 border-t border-muted-foreground"),
    },
});

const starterKit = StarterKit.configure({
    bulletList: {
        HTMLAttributes: {
            class: cx("list-disc list-outside leading-3 -mt-2"),
        },
    },
    orderedList: {
        HTMLAttributes: {
            class: cx("list-decimal list-outside leading-3 -mt-2"),
        },
    },
    listItem: {
        HTMLAttributes: {
            class: cx("leading-normal -mb-2"),
        },
    },
    blockquote: {
        HTMLAttributes: {
            class: cx("border-l-4 border-primary"),
        },
    },
    codeBlock: false,
    code: {
        HTMLAttributes: {
            class: cx(
                "rounded-md bg-muted px-1.5 py-1 font-mono font-medium"
            ),
            spellcheck: "false",
        },
    },
    horizontalRule: false,
    dropcursor: {
        color: "#DBEAFE",
        width: 4,
    },
});

const codeBlockLowlight = CodeBlockLowlight.configure({
    lowlight: createLowlight(common),
    HTMLAttributes: {
        class: cx(
            "rounded-md bg-muted text-muted-foreground border p-5 font-mono font-medium"
        ),
    },
});

const table = Table.configure({
    resizable: true,
    HTMLAttributes: {
        class: cx("not-prose m-0"),
    },
});

const tableRow = TableRow.configure({
    HTMLAttributes: {},
});

const tableCell = TableCell.configure({
    HTMLAttributes: {
        class: cx("border border-border px-3 py-1.5 align-top text-sm relative"),
    },
});

const tableHeader = TableHeader.configure({
    HTMLAttributes: {
        class: cx("border border-border px-3 py-1.5 font-medium bg-muted/50 align-top text-sm"),
    },
});

const image = Image.configure({
    HTMLAttributes: {
        class: cx("rounded-lg max-w-full h-auto my-4"),
    },
    allowBase64: true,
});

export const defaultExtensions: Extensions = [
    starterKit,
    tiptapLink,
    taskList,
    taskItem,
    horizontalRule,
    codeBlockLowlight,
    table,
    tableRow,
    tableCell,
    tableHeader,
    image,
    VideoExtension,
    AudioExtension,
    FileAttachmentExtension,
    UploadPlaceholderExtension,
    ...htmlExtensions,
    Markdown.configure({
        html: true,
        transformPastedText: true,
        transformCopiedText: true,
    }),
    TiptapUnderline,
    HighlightExtension,
    TextStyle,
    Color,
    CustomKeymap,
    TrailingNode,
    GlobalDragHandle.configure({
        dragHandleWidth: 20,
        scrollTreshold: 100,
        excludedTags: ["table", "tbody", "thead", "tfoot", "tr", "td", "th", "video", "audio", "iframe"],
    }),
    FixDragHandleDrop,
];
