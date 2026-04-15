# @mydrift/runesh-ui

Shared React/Next.js UI components, hooks, fonts, and client helpers for RUNESH projects.

## Install

```bash
bun add @mydrift/runesh-ui
```

Peer dependencies (install in the consuming app):

```bash
bun add react react-dom next next-themes lucide-react novel lowlight \
  @tiptap/core @tiptap/pm \
  @tiptap/extension-image @tiptap/extension-table @tiptap/extension-table-cell \
  @tiptap/extension-table-header @tiptap/extension-table-row \
  tiptap-extension-global-drag-handle tiptap-markdown
```

## Usage

```ts
import { cn, FONT_FAMILY_SANS } from "@mydrift/runesh-ui";
import { AppSidebar, DashboardShell } from "@mydrift/runesh-ui/components";
import { useIsMobile } from "@mydrift/runesh-ui/hooks";
import { apiClient } from "@mydrift/runesh-ui/lib";
```

Global styles (Tailwind + OKLCH theme):

```ts
import "@mydrift/runesh-ui/styles/globals.css";
```

## Subpath exports

| Import | Contents |
|---|---|
| `@mydrift/runesh-ui` | Fonts, lib, hooks (re-exported) |
| `@mydrift/runesh-ui/components` | Layout, auth, editor, providers, ui |
| `@mydrift/runesh-ui/lib` | `cn`, api client, token store, auth-pkce, formatting |
| `@mydrift/runesh-ui/hooks` | `useIsMobile`, `useTauri`, `useWebSocket`, etc. |
| `@mydrift/runesh-ui/fonts` | Chiron GoRound TC font constants |
| `@mydrift/runesh-ui/styles/globals.css` | Base Tailwind layer + theme |

## License

MIT OR Apache-2.0
