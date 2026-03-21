import { Extension } from "@tiptap/core";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

export const searchHighlightKey = new PluginKey<DecorationSet>("wikiSearchHighlight");

/**
 * Tiptap extension that adds inline decorations (.wiki-search-highlight) for
 * every occurrence of a search term in the document.
 *
 * Usage:
 *   editor.view.dispatch(
 *     editor.state.tr.setMeta(searchHighlightKey, "term")   // set
 *     editor.state.tr.setMeta(searchHighlightKey, "")       // clear
 *   );
 */
export const SearchHighlightExtension = Extension.create({
    name: "wikiSearchHighlight",
    addProseMirrorPlugins() {
        return [
            new Plugin({
                key: searchHighlightKey,
                state: {
                    init() {
                        return DecorationSet.empty;
                    },
                    apply(tr, old) {
                        const meta = tr.getMeta(searchHighlightKey) as
                            | { term: string; occurrenceIndex?: number }
                            | string
                            | undefined;
                        if (meta === undefined) return old.map(tr.mapping, tr.doc);

                        const term = typeof meta === "string" ? meta : (meta?.term ?? "");
                        const occIndex = typeof meta === "object" && meta !== null ? meta.occurrenceIndex : undefined;

                        if (!term) return DecorationSet.empty;

                        const lower = term.toLowerCase();
                        const decorations: Decoration[] = [];
                        let occurrence = 0;

                        tr.doc.descendants((node, pos) => {
                            if (!node.isText || !node.text) return;
                            const text = node.text.toLowerCase();
                            let idx = text.indexOf(lower);
                            while (idx !== -1) {
                                if (occIndex === undefined || occurrence === occIndex) {
                                    decorations.push(
                                        Decoration.inline(pos + idx, pos + idx + term.length, {
                                            class: "wiki-search-highlight",
                                        })
                                    );
                                }
                                occurrence++;
                                idx = text.indexOf(lower, idx + 1);
                            }
                        });

                        return DecorationSet.create(tr.doc, decorations);
                    },
                },
                props: {
                    decorations(state) {
                        return searchHighlightKey.getState(state) ?? DecorationSet.empty;
                    },
                },
            }),
        ];
    },
});
