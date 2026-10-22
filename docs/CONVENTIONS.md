# Conventions & standard procedures

How code in this repo should look, and the step-by-step for common changes.

## Everyday commands

| Command | What it does |
|---|---|
| `npm install` | install JS deps (once) |
| `npm run tauri dev` | run the app with hot reload |
| `npm run bindings` | regenerate `ui/src/lib/bindings.ts` after IPC changes |
| `npm run check` / `npm test` | UI typecheck / UI tests |
| `cargo test --manifest-path src-tauri/Cargo.toml` | Rust tests |
| `npx tauri build` | release installer (on Windows) |
| `npx tauri icon src-tauri/icons/app-icon.png -o src-tauri/icons` | regenerate icons |
| `scripts/check-windows.sh` | lint the Windows build from Linux (`rustup target add x86_64-pc-windows-msvc` once); fakes the C compiler for bundled SQLite since lint never links |

Debugging tips:
- `DA_KEEP_OPEN=1` stops the launcher hiding when it loses focus (for devtools).
- `RUST_LOG=desktop_assistant_lib=debug` logs timings such as hotkey-to-visible.
- On Linux the webview won't shrink below 200 px tall, so the bar-only window
  looks taller there than on Windows. Not worth working around.

## General

- Small modules with one job. If a file passes ~400 lines, split it.
- Names say what things are: `everything.rs`, not `search2.rs`.
- Comments explain *why*, not what. No commented-out code.
- No new dependency without a reason in the commit message.
- Personal app: skip anything that only matters for other users
  (telemetry, localization, multi-tenant config, license checks).

## Rust

- `cargo fmt` and `cargo clippy -- -D warnings` must pass.
- Errors: `thiserror` enums per module, converted into `AppError` in
  `error.rs`. `anyhow` only in `main.rs`/setup code.
- No `unwrap()`/`expect()` outside tests and startup code where failure means
  the app can't run anyway. Never panic on the search or AI path.
- Async: `tokio`. Never block the async runtime; use `spawn_blocking` for
  Win32 or Everything calls that can stall.
- Windows-only code goes in `platform/windows/` or behind `#[cfg(windows)]`
  with a stub for other targets, so `cargo test` runs on Linux.
- Logging with `tracing`. `info` for lifecycle events, `debug` for per-query
  detail. Never log API keys. Prompts are logged only at `trace`.

## UI (Svelte + TS)

- `strict` TypeScript. Call the backend only through generated
  `lib/bindings.ts`. Never use raw `invoke("name")` strings.
- Components are presentational; state lives in `lib/stores/` as Svelte 5
  rune classes in `*.svelte.ts` files.
- Colors, radii, blur and spacing come from `styles/tokens.css` only.
- Every interactive element is reachable by keyboard.

## IPC

- Commands (request/response) are named `verb_noun`: `search_query`,
  `ai_send`, `settings_get`.
- Events (backend to UI streams) are named `noun:verb`: `ai:delta`,
  `ai:approval`, `index:progress`.
- Every payload is a Rust struct with `#[derive(Serialize, specta::Type)]`.
  Regenerate bindings after changing one (`npm run bindings`). A Rust test
  fails if the committed bindings are stale.
- Avoid `u64`/`i64` and floats in IPC types (TypeScript can't hold u64 safely,
  and floats become `number | null`). Use `u32` or keep the field Rust-only
  with `#[serde(skip)]` + `#[specta(skip)]`.
- Settings structs don't use `#[serde(default)]`; defaults come from
  `Settings::default()` via layering, which keeps generated types non-optional.

## Procedure: add a feature (D-027)

1. Work in `src-tauri/src/features/<name>/` (the folder and its line in
   `features/mod.rs` already exist for planned features). `provider(ctx)`
   returns your `SearchProvider`; `start(app)` starts background work.
2. Results: build them with `ResultItem::new(..).with_…`. Use an id prefix
   only you make (`"calc:"`) and return true from `owns` for it, so
   activations come to your `activate`. Return raw scores; the ranker
   applies frecency. Return true from `claims` only for a prefix meant for
   you alone (`kill `, `clip`).
3. Icons: `glyph("name")` (add new ones to `ui/src/lib/glyphs.ts`),
   `text_icon("🔥")`, `color_icon("#ff8800")`, or `thumb_icon(name)` for a
   PNG you wrote to `AppPaths::thumbnails_dir()`.
4. `activate` does the work and returns what follows (`Activation`). Use
   `Copy`/`Paste`/`OpenUrl` rather than doing those yourself. Risky
   commands set `with_confirm("Restart your PC?")`.
5. Settings: grow your section's struct in your folder (every field needs
   a default). A setting that only makes sense on one PC (a path on its
   disk, how its Windows behaves) also goes in `settings::MACHINE_KEYS`,
   so a synced `settings.toml` never carries it to another PC. UI: fill in
   `components/settings/<Name>Section.svelte`.
6. Windows-only code: `#[cfg(windows)] mod win;` in your folder, with a
   stub path so `cargo test` passes on Linux.
7. Commands/events the UI needs go in `ipc/mod.rs` under your `// [name]`
   marker; then `npm run bindings`.
8. Unit-test parsing, matching and id stability. Keep `query` inside the
   40 ms budget (do slow work in `start` and cache it).

## Procedure: add an AI provider

Follow `ai/claude/` as the model.
1. Create `ai/<name>/mod.rs`: find the CLI, build its command line.
2. Write the output parser as a pure function in `ai/<name>/protocol.rs`
   (`fn parse_line(&str) -> Vec<Incoming>`), producing `AiEvent`s.
3. Record real output into `src-tauri/tests/fixtures/<name>/*.jsonl`
   (strip paths and ids) and test the parser against it.
4. Handle billing mode: strip or inject the provider's API key env var.
5. Map approvals into `AiEvent::ApprovalRequested`, and the answer back.
6. Hook it into `chat.rs`, the provider switcher and the prefix map.

## Procedure: add a built-in command

1. Add a variant in `search/commands.rs` with a title, keywords and handler.
2. If it needs settings, add fields with defaults to `Settings`.
3. Test that the keywords match.

## Testing

| Layer | What | Where |
|---|---|---|
| Unit | ranking, routing/prefixes, parsers, settings migration | inline `#[cfg(test)]` |
| Fixture | Claude/Codex stream parsing from recorded output | `src-tauri/tests/` |
| UI | stores and keyboard handling (Vitest) | `ui/src/**/*.test.ts` |
| Manual | hotkey, glass, Win key, real search | checklist in PR/commit message |

Not worth it here: end-to-end UI automation, mocking Win32.

## Git

- Branch per piece of work; small commits with imperative messages
  ("Add Everything search provider").
- A milestone isn't done until its ROADMAP boxes are ticked and CI is green.

## CI / release

- `test.yml`: UI typecheck/tests/build on Linux; Rust fmt, clippy and tests
  on both Linux and Windows (the Windows job covers the keyboard hook).
- `build.yml` (windows-latest, every push): `tauri build` produces an
  `.msi`/`.exe` installer uploaded as a workflow artifact. These builds
  have no update key, so they don't update themselves.
- `release.yml` (tags `v*`, or run by hand): builds the NSIS installer,
  signs it for the updater and publishes a GitHub Release with
  `latest.json`, which installed copies check. Without the signing secrets
  it still publishes the installer, with a warning and no update files.
- Releasing: set the same version in `src-tauri/tauri.conf.json`,
  `src-tauri/Cargo.toml` and `package.json` (the workflow checks), commit,
  then tag `v<version>` and push the tag.

## Settings sync across my computers

`settings.toml` and the MCP registry are plain text with no secrets, so they
can live in a synced folder (OneDrive; Settings → Advanced → Settings
folder). Each PC's own values (`MACHINE_KEYS`) stay in
`%LOCALAPPDATA%\desktop-assistant\settings.local.toml`, and keys stay per
machine in Credential Manager.
