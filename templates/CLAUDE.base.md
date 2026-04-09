## Workflow Rules

- **Every change MUST go through a Pull Request (PR).** No direct commits to `main`.
- **Before creating a PR, always merge `main` into the feature branch** to ensure the latest changes are included.
- **Every frontend change MUST be verified using the Playwright MCP** before committing.
- **Always use `bun`.** Never use `npm` or `yarn`.
- **Always test changes end-to-end** (docker rebuild, playwright) before claiming they work. Compiling is not testing.

## Attribution

- **Never add Claude/AI as co-author, contributor, or any form of attribution in commits, PRs, or code.**

## AI Assistant Rules

- **Never do more than what was asked.** Do not add features, configs, gitignore rules, or "improvements" that were not explicitly requested. If in doubt, ask first.
- **Never modify infrastructure (docker, CI, env, volumes, databases) without explicit confirmation.** Changing volume names, postgres versions, or compose structure can destroy data.
- **Never commit screenshots, test artifacts, or temporary files** to the repo. Clean up after yourself but do not modify .gitignore unless asked.
- **Never use em dashes or en dashes** anywhere in commits, release notes, or prose. Rewrite the sentence to avoid them entirely.
- **Keep release bodies short.** No filler prose, no walls of text.

## Writing Pull Requests

- Title starts with a past-tense verb: `Added`, `Fixed`, `Refactored`, etc.
- Body MUST contain `resolve #<issue-number>` to auto-close the linked issue (if applicable)
- One PR per issue. Keep changes focused.
- Include a `## Summary` with bullet points and a `## Test plan`
- Squash merge to main

## Labels

Every issue and PR must have a label: `bug`, `enhancement`, `feature`, `refactor`
