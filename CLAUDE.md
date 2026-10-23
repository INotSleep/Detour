# Detour

Open-source serverless SSH client (macOS, Windows, Linux) with E2E-encrypted Teams.
Stack: Tauri 2 + Rust (`crates/detour-core` — UI-agnostic core, `src-tauri` — thin shell) + React/TS + xterm.js.
`detour-core` never depends on Tauri. The UI talks to the backend only through `src/lib/api.ts`.

UI languages: English and Russian. Every UI string lives in `src/i18n/<area>.ts` (`en` and `ru` of one shape, tsc checks both); components read it with `useT()`, other code with `tr()`. Never hard-code UI text or keep translated text at module scope. Discussions with the user are in Russian.

## UI/UX rules (mandatory)

1. **User-friendly.** The main action on every screen is obvious and takes the fewest steps. Sensible defaults in forms. Keyboard works everywhere (Enter submits, Esc closes, Ctrl+K quick connect). Prefer undo over "are you sure?"; confirm only actions that cannot be undone.
2. **Every action gets a visible reaction.** Hover/pressed/focus states on everything clickable. Anything that takes time shows progress. Success and errors are shown (toast, inline state). No dead controls: if a feature doesn't work yet, the control is not shown at all.
3. **No text without a function.** Text is only for labels, values, states, errors and hints that tell you what to do right now. No explanatory subtitles, no descriptions of how encryption/sync/integrity works, no marketing — that belongs in docs and the website.
4. **No long dashes (— or –) in UI text.** The UI font lacks them and they render narrow. Rephrase, or use a colon, parentheses or a plain hyphen.
5. **Design quality matters.** No mock/debug markers in the UI ("макет", "mock", "симуляция", etc.) — the design is judged as if it were the real product. Check every changed screen with a screenshot at a real window size before reporting it done.
