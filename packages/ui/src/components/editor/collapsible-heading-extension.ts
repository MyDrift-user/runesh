import { Extension } from "@tiptap/core";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

export const collapsibleKey = new PluginKey<Set<number>>("collapsibleHeading");

// Inline SVG carets
const CARET_RIGHT = `<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8" viewBox="0 0 8 8" fill="currentColor"><path d="M2 1.5l3.5 2.5L2 6.5V1.5z"/></svg>`;
const CARET_DOWN  = `<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8" viewBox="0 0 8 8" fill="currentColor"><path d="M1.5 2.5l2.5 3.5 2.5-3.5H1.5z"/></svg>`;

interface HeadingInfo { pos: number; level: number; size: number }

function buildDecorations(doc: any, collapsed: Set<number>): DecorationSet {
    const decorations: Decoration[] = [];

    // Collect all top-level headings
    const headings: HeadingInfo[] = [];
    doc.forEach((node: any, offset: number) => {
        if (node.type.name === "heading") {
            headings.push({ pos: offset, level: node.attrs.level as number, size: node.nodeSize });
        }
    });

    for (const { pos, level, size } of headings) {
        const isCollapsed = collapsed.has(pos);

        // Toggle button widget placed at the very start of the heading content
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = "wiki-collapse-toggle" + (isCollapsed ? " is-collapsed" : "");
        btn.setAttribute("data-heading-pos", String(pos));
        btn.setAttribute("contenteditable", "false");
        btn.setAttribute("tabindex", "-1");
        btn.innerHTML = isCollapsed ? CARET_RIGHT : CARET_DOWN;

        decorations.push(
            Decoration.widget(pos + 1, btn, {
                side: -1,
                key: `collapse-${pos}`,
                stopEvent: () => true,
            })
        );

        if (isCollapsed) {
            // Hide all blocks after the heading up to the next heading of same or higher level
            const afterHeading = pos + size;
            let rangeEnd = doc.content.size;
            for (const other of headings) {
                if (other.pos >= afterHeading && other.level <= level) {
                    rangeEnd = other.pos;
                    break;
                }
            }

            doc.forEach((node: any, offset: number) => {
                if (offset >= afterHeading && offset < rangeEnd) {
                    decorations.push(
                        Decoration.node(offset, offset + node.nodeSize, {
                            class: "wiki-collapsed-block",
                        })
                    );
                }
            });
        }
    }

    return DecorationSet.create(doc, decorations);
}

export const CollapsibleHeadingExtension = Extension.create({
    name: "collapsibleHeading",

    addProseMirrorPlugins() {
        return [
            new Plugin({
                key: collapsibleKey,

                state: {
                    init: () => new Set<number>(),
                    apply(tr, prev) {
                        const meta = tr.getMeta(collapsibleKey) as number | undefined;
                        if (meta !== undefined) {
                            const next = new Set(prev);
                            next.has(meta) ? next.delete(meta) : next.add(meta);
                            return next;
                        }
                        if (!tr.docChanged) return prev;
                        // Remap stored positions through the document change
                        const next = new Set<number>();
                        prev.forEach((pos) => next.add(tr.mapping.map(pos)));
                        return next;
                    },
                },

                props: {
                    decorations(state) {
                        return buildDecorations(state.doc, collapsibleKey.getState(state)!);
                    },

                    handleDOMEvents: {
                        mousedown(view, event) {
                            const target = (event.target as HTMLElement).closest(
                                "[data-heading-pos]"
                            ) as HTMLElement | null;
                            if (!target?.dataset.headingPos) return false;
                            const pos = parseInt(target.dataset.headingPos);
                            event.preventDefault();
                            view.dispatch(view.state.tr.setMeta(collapsibleKey, pos));
                            return true;
                        },
                    },
                },
            }),
        ];
    },
});
