"use client";

import { useEffect, useState, useCallback, useRef } from "react";
import { Plus, Minus, Trash2, Columns3, Rows3 } from "lucide-react";

interface TableMenuProps {
    editor: {
        isActive: (name: string) => boolean;
        chain: () => {
            focus: () => {
                addColumnAfter: () => { run: () => void };
                addRowAfter: () => { run: () => void };
                deleteColumn: () => { run: () => void };
                deleteRow: () => { run: () => void };
                deleteTable: () => { run: () => void };
                toggleHeaderRow: () => { run: () => void };
                toggleHeaderColumn: () => { run: () => void };
            };
        };
        on: (event: string, cb: () => void) => void;
        off: (event: string, cb: () => void) => void;
        view: {
            dom: HTMLElement;
        };
    };
    scrollContainer: HTMLElement | null;
}

export function TableMenu({ editor, scrollContainer }: TableMenuProps) {
    const [isInTable, setIsInTable] = useState(false);
    const [position, setPosition] = useState<{ top: number; left: number } | null>(null);
    const menuRef = useRef<HTMLDivElement>(null);

    const updatePosition = useCallback(() => {
        const inTable = editor.isActive("table");
        setIsInTable(inTable);

        if (!inTable || !scrollContainer) {
            setPosition(null);
            return;
        }

        // Find the active table element in the editor DOM
        const tableEl = editor.view.dom.querySelector(".ProseMirror-selectednode table")
            || editor.view.dom.querySelector("table:has(.selectedCell)")
            || (() => {
                // Fallback: find table containing the cursor
                const sel = window.getSelection();
                if (!sel?.anchorNode) return null;
                let node: Node | null = sel.anchorNode;
                while (node && node !== editor.view.dom) {
                    if (node instanceof HTMLElement && node.tagName === "TABLE") return node;
                    if (node instanceof HTMLElement && node.classList?.contains("tableWrapper")) {
                        return node.querySelector("table");
                    }
                    node = node.parentNode;
                }
                return null;
            })();

        if (!tableEl) {
            setPosition(null);
            return;
        }

        const tableRect = tableEl.getBoundingClientRect();
        const containerRect = scrollContainer.getBoundingClientRect();

        setPosition({
            top: tableRect.top - containerRect.top + scrollContainer.scrollTop - 32,
            left: tableRect.left - containerRect.left,
        });
    }, [editor, scrollContainer]);

    useEffect(() => {
        editor.on("selectionUpdate", updatePosition);
        editor.on("transaction", updatePosition);

        const onScroll = () => {
            if (isInTable) updatePosition();
        };
        scrollContainer?.addEventListener("scroll", onScroll, { passive: true });

        return () => {
            editor.off("selectionUpdate", updatePosition);
            editor.off("transaction", updatePosition);
            scrollContainer?.removeEventListener("scroll", onScroll);
        };
    }, [editor, updatePosition, scrollContainer, isInTable]);

    if (!isInTable || !position) return null;

    const btn =
        "inline-flex items-center gap-1 px-2 py-1 rounded text-xs font-medium transition-colors hover:bg-accent text-muted-foreground hover:text-foreground whitespace-nowrap cursor-pointer";
    const sep = "w-px h-4 bg-border mx-0.5";

    return (
        <div
            ref={menuRef}
            className="absolute z-20 pointer-events-auto"
            style={{ top: position.top, left: position.left }}
        >
            <div className="inline-flex items-center gap-0.5 rounded-md border border-border bg-background/95 backdrop-blur-sm shadow-md px-1 py-0.5">
                <button onClick={() => editor.chain().focus().addRowAfter().run()} className={btn} title="Add row">
                    <Plus size={12} /> Row
                </button>
                <button onClick={() => editor.chain().focus().addColumnAfter().run()} className={btn} title="Add column">
                    <Plus size={12} /> Col
                </button>
                <div className={sep} />
                <button onClick={() => editor.chain().focus().deleteRow().run()} className={btn} title="Delete row">
                    <Minus size={12} /> Row
                </button>
                <button onClick={() => editor.chain().focus().deleteColumn().run()} className={btn} title="Delete column">
                    <Minus size={12} /> Col
                </button>
                <div className={sep} />
                <button onClick={() => editor.chain().focus().toggleHeaderRow().run()} className={btn} title="Toggle header row">
                    <Rows3 size={12} /> Header
                </button>
                <button onClick={() => editor.chain().focus().toggleHeaderColumn().run()} className={btn} title="Toggle header column">
                    <Columns3 size={12} /> Header
                </button>
                <div className={sep} />
                <button onClick={() => editor.chain().focus().deleteTable().run()} className={`${btn} hover:text-red-400`} title="Delete table">
                    <Trash2 size={12} />
                </button>
            </div>
        </div>
    );
}
