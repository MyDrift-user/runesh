import { defineConfig } from "tsup";

export default defineConfig({
  entry: {
    index: "src/index.ts",
    components: "src/components/index.ts",
    lib: "src/lib/index.ts",
    hooks: "src/hooks/index.ts",
    fonts: "src/fonts/index.ts",
  },
  format: ["esm"],
  dts: true,
  sourcemap: true,
  clean: true,
  target: "es2022",
  treeshake: true,
  external: [
    "react",
    "react-dom",
    "next",
    "next-themes",
    "novel",
    "lowlight",
    "lucide-react",
    /^@tiptap\//,
    "tiptap-extension-global-drag-handle",
    "tiptap-markdown",
  ],
  tsconfig: "tsconfig.json",
});
