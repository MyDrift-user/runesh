"use client";

import { cn } from "../../lib/utils";
import {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Code,
    Highlighter,
} from "lucide-react";
import { EditorBubble, EditorBubbleItem, useEditor } from "novel";
import type { ReactNode } from "react";

interface TextButton {
    name: string;
    icon: ReactNode;
    command: (editor: NonNullable<ReturnType<typeof useEditor>["editor"]>) => void;
    isActive: (editor: NonNullable<ReturnType<typeof useEditor>["editor"]>) => boolean;
}

const textButtons: TextButton[] = [
    {
        name: "Bold",
        icon: <Bold className="h-4 w-4" />,
        command: (editor) => editor.chain().focus().toggleBold().run(),
        isActive: (editor) => editor.isActive("bold"),
    },
    {
        name: "Italic",
        icon: <Italic className="h-4 w-4" />,
        command: (editor) => editor.chain().focus().toggleItalic().run(),
        isActive: (editor) => editor.isActive("italic"),
    },
    {
        name: "Underline",
        icon: <Underline className="h-4 w-4" />,
        command: (editor) => editor.chain().focus().toggleUnderline().run(),
        isActive: (editor) => editor.isActive("underline"),
    },
    {
        name: "Strikethrough",
        icon: <Strikethrough className="h-4 w-4" />,
        command: (editor) => editor.chain().focus().toggleStrike().run(),
        isActive: (editor) => editor.isActive("strike"),
    },
    {
        name: "Code",
        icon: <Code className="h-4 w-4" />,
        command: (editor) => editor.chain().focus().toggleCode().run(),
        isActive: (editor) => editor.isActive("code"),
    },
    {
        name: "Highlight",
        icon: <Highlighter className="h-4 w-4" />,
        command: (editor) => editor.chain().focus().toggleHighlight().run(),
        isActive: (editor) => editor.isActive("highlight"),
    },
];

export function TextButtons() {
    const { editor } = useEditor();
    if (!editor) return null;

    return (
        <div className="flex items-center">
            {textButtons.map((item) => (
                <EditorBubbleItem
                    key={item.name}
                    onSelect={(editor) => item.command(editor)}
                >
                    <button
                        type="button"
                        className={cn(
                            "p-2 text-muted-foreground hover:text-foreground transition-colors rounded-sm",
                            item.isActive(editor) && "bg-accent text-foreground"
                        )}
                        title={item.name}
                    >
                        {item.icon}
                    </button>
                </EditorBubbleItem>
            ))}
        </div>
    );
}

export function EditorBubbleMenu() {
    return (
        <EditorBubble
            tippyOptions={{
                placement: "top",
            }}
            className="flex w-fit max-w-[90vw] overflow-hidden rounded-lg border border-muted bg-background shadow-xl"
        >
            <TextButtons />
        </EditorBubble>
    );
}
