# CLAUDE.md

Personal Raycast-style launcher for Windows (Tauri 2 + Rust + Svelte 5).
Single user, personal machines only.

Before changing code, read:
- `docs/ARCHITECTURE.md`: module layout, core traits, data flow, budgets
- `docs/CONVENTIONS.md`: style rules and step-by-step procedures
- `docs/DECISIONS.md`: why things are the way they are (add entries, don't rewrite)
- `docs/ROADMAP.md`: tick boxes when work lands

Key rules:
- Windows-only code goes behind `#[cfg(windows)]` or in `platform/windows/`;
  `cargo test` must pass on Linux.
- UI talks to Rust only through generated `ui/src/lib/bindings.ts`.
- No panics on the search or AI paths. Never log API keys.
- Subscription mode must strip `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` from
  child process env.
- Colors and spacing come from `ui/src/styles/tokens.css` only.
