import {
    CheckSquare,
    Code,
    Heading1,
    Heading2,
    Heading3,
    Heading4,
    List,
    ListOrdered,
    Text,
    TextQuote,
    Minus,
    Table,
    ImageIcon,
    Video,
    Music,
    Paperclip,
} from "lucide-react";
import { createSuggestionItems, Command, renderItems } from "novel";

export const suggestionItems = createSuggestionItems([
    {
        title: "Text",
        description: "Plain text paragraph.",
        searchTerms: ["p", "paragraph"],
        icon: <Text size={18} />,
        command: ({ editor, range }) => {
            editor
                .chain()
                .focus()
                .deleteRange(range)
                .toggleNode("paragraph", "paragraph")
                .run();
        },
    },
    {
        title: "Heading 1",
        description: "Large section heading.",
        searchTerms: ["title", "big", "large", "h1"],
        icon: <Heading1 size={18} />,
        command: ({ editor, range }) => {
            editor
                .chain()
                .focus()
                .deleteRange(range)
                .setNode("heading", { level: 1 })
                .run();
        },
    },
    {
        title: "Heading 2",
        description: "Medium section heading.",
        searchTerms: ["subtitle", "medium", "h2"],
        icon: <Heading2 size={18} />,
        command: ({ editor, range }) => {
            editor
                .chain()
                .focus()
                .deleteRange(range)
                .setNode("heading", { level: 2 })
                .run();
        },
    },
    {
        title: "Heading 3",
        description: "Small section heading.",
        searchTerms: ["subtitle", "small", "h3"],
        icon: <Heading3 size={18} />,
        command: ({ editor, range }) => {
            editor
                .chain()
                .focus()
                .deleteRange(range)
                .setNode("heading", { level: 3 })
                .run();
        },
    },
    {
        title: "Heading 4",
        description: "Subsection heading.",
        searchTerms: ["h4"],
        icon: <Heading4 size={18} />,
        command: ({ editor, range }) => {
            editor
                .chain()
                .focus()
                .deleteRange(range)
                .setNode("heading", { level: 4 })
                .run();
        },
    },
    {
        title: "Bullet List",
        description: "Unordered list of items.",
        searchTerms: ["unordered", "point", "ul"],
        icon: <List size={18} />,
        command: ({ editor, range }) => {
            editor.chain().focus().deleteRange(range).toggleBulletList().run();
        },
    },
    {
        title: "Numbered List",
        description: "Ordered list with numbers.",
        searchTerms: ["ordered", "ol"],
        icon: <ListOrdered size={18} />,
        command: ({ editor, range }) => {
            editor.chain().focus().deleteRange(range).toggleOrderedList().run();
        },
    },
    {
        title: "To-do List",
        description: "Track tasks with checkboxes.",
        searchTerms: ["todo", "task", "check", "checkbox"],
        icon: <CheckSquare size={18} />,
        command: ({ editor, range }) => {
            editor.chain().focus().deleteRange(range).toggleTaskList().run();
        },
    },
    {
        title: "Table",
        description: "Insert a table with rows and columns.",
        searchTerms: ["table", "grid", "spreadsheet", "rows", "columns"],
        icon: <Table size={18} />,
        command: ({ editor, range }) => {
            editor
                .chain()
                .focus()
                .deleteRange(range)
                .insertTable({ rows: 3, cols: 3, withHeaderRow: true })
                .run();
        },
    },
    {
        title: "Quote",
        description: "Capture a blockquote.",
        searchTerms: ["blockquote", "cite", "quote"],
        icon: <TextQuote size={18} />,
        command: ({ editor, range }) =>
            editor
                .chain()
                .focus()
                .deleteRange(range)
                .toggleNode("paragraph", "paragraph")
                .toggleBlockquote()
                .run(),
    },
    {
        title: "Code Block",
        description: "Syntax-highlighted code.",
        searchTerms: ["codeblock", "code", "pre", "```"],
        icon: <Code size={18} />,
        command: ({ editor, range }) =>
            editor.chain().focus().deleteRange(range).toggleCodeBlock().run(),
    },
    {
        title: "Divider",
        description: "Horizontal separator line.",
        searchTerms: ["hr", "divider", "separator", "line"],
        icon: <Minus size={18} />,
        command: ({ editor, range }) => {
            editor.chain().focus().deleteRange(range).setHorizontalRule().run();
        },
    },
    {
        title: "Image",
        description: "Upload or embed an image.",
        searchTerms: ["image", "photo", "picture", "img"],
        icon: <ImageIcon size={18} />,
        command: ({ editor, range }) => {
            editor.chain().focus().deleteRange(range).run();
            const input = document.createElement("input");
            input.type = "file";
            input.accept = "image/*";
            input.onchange = () => {
                const file = input.files?.[0];
                if (file) {
                    // Triggers the FileHandlerExtension's onUpload if configured
                    const event = new CustomEvent("editor-file-upload", { detail: { file } });
                    document.dispatchEvent(event);
                }
            };
            input.click();
        },
    },
    {
        title: "Video",
        description: "Upload a video file.",
        searchTerms: ["video", "movie", "mp4", "clip"],
        icon: <Video size={18} />,
        command: ({ editor, range }) => {
            editor.chain().focus().deleteRange(range).run();
            const input = document.createElement("input");
            input.type = "file";
            input.accept = "video/*";
            input.onchange = () => {
                const file = input.files?.[0];
                if (file) {
                    const event = new CustomEvent("editor-file-upload", { detail: { file } });
                    document.dispatchEvent(event);
                }
            };
            input.click();
        },
    },
    {
        title: "Audio",
        description: "Upload an audio file.",
        searchTerms: ["audio", "sound", "music", "mp3"],
        icon: <Music size={18} />,
        command: ({ editor, range }) => {
            editor.chain().focus().deleteRange(range).run();
            const input = document.createElement("input");
            input.type = "file";
            input.accept = "audio/*";
            input.onchange = () => {
                const file = input.files?.[0];
                if (file) {
                    const event = new CustomEvent("editor-file-upload", { detail: { file } });
                    document.dispatchEvent(event);
                }
            };
            input.click();
        },
    },
    {
        title: "File",
        description: "Attach any file (PDF, document, etc).",
        searchTerms: ["file", "attachment", "pdf", "document", "upload"],
        icon: <Paperclip size={18} />,
        command: ({ editor, range }) => {
            editor.chain().focus().deleteRange(range).run();
            const input = document.createElement("input");
            input.type = "file";
            input.onchange = () => {
                const file = input.files?.[0];
                if (file) {
                    const event = new CustomEvent("editor-file-upload", { detail: { file } });
                    document.dispatchEvent(event);
                }
            };
            input.click();
        },
    },
]);

export const slashCommand = Command.configure({
    suggestion: {
        items: () => suggestionItems,
        render: renderItems,
    },
});
