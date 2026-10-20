# Roadmap

Milestones are in build order. Each one should leave the app usable.
Tick boxes as work lands.

## M0 Project setup ✅
- [x] Scaffold Tauri 2 + Svelte 5 + TS (`src-tauri/`, `ui/`)
- [x] Core types and traits (`AppError`, `SearchProvider` + aggregator, `AiProvider`, `AiEvent`)
- [x] `tauri-specta` bindings generation + `gen-bindings` bin + staleness test
- [x] `tracing` to rolling log file
- [x] `Settings` struct with defaults, TOML load/save, version + migration hook
- [x] Built-in commands (Open Settings, Open Log Folder, Quit) end to end
- [x] CI: `test.yml` (ubuntu) and `build.yml` (windows installer artifact)
- [x] `CLAUDE.md` / README kept in sync with docs

## M1 Shell: window, hotkeys, glass
- [ ] Frameless, transparent, always-on-top window created hidden at startup
- [ ] Center on the monitor with the cursor; remember size
- [ ] Acrylic via `window-vibrancy`; Win10 fallback tint
- [ ] Glass design tokens + SearchBar + ResultList shell
- [ ] Ctrl+Space global shortcut toggles the window
- [ ] **Windows-key tap override** (`WH_KEYBOARD_LL`, combos pass through, dummy key to suppress Start)
- [ ] Hide on blur, on Esc, and after launching
- [ ] Tray icon: Settings, Reindex, Quit
- [ ] Single-instance guard
- [ ] Start with Windows (setting)
- [ ] Light/dark follows Windows; reduce-motion respected
- [ ] Measure hotkey-to-visible (< 50 ms)

## M2 App launcher
- [ ] Index Start Menu `.lnk` (user + all users) and Store apps (`shell:AppsFolder`)
- [ ] Icon extraction + on-disk icon cache
- [ ] `nucleo` fuzzy matching, acronyms ("vsc" → Visual Studio Code)
- [ ] SQLite store + frecency recording
- [ ] Ranker: fuzzy × frecency, exact-prefix pin
- [ ] Launch via ShellExecute; "Run as administrator" secondary action
- [ ] Re-index when Start Menu folders change (file watcher)
- [ ] Keyboard nav: ↑/↓, Enter, Ctrl+1..8, Esc

## M3 File search (Everything)
- [ ] Everything SDK wrapper (64-bit DLL bundled), in `spawn_blocking`
- [ ] Detect missing or stopped Everything and show a fix-it row
- [ ] Merge file/folder results with apps; files rank below strong app matches
- [ ] Exclusions (`node_modules`, `.git`, `AppData` temp, `$Recycle.Bin`) in settings
- [ ] File actions: Open, Reveal in Explorer, Copy path, Open with…, Delete to Recycle Bin
- [ ] Filters: `f ` files only, `d ` folders only, `ext:pdf`
- [ ] Query-sequence handling so stale results never flash

## M4 Claude
- [ ] Detect `claude` install + login status; fix-it row if missing
- [ ] Routing: no matches + Enter → AI; Tab → AI; `c ` prefix; "Ask Claude" row always last
- [ ] Long-lived stream-json child process; warm spare process
- [ ] Strip `ANTHROPIC_API_KEY` in subscription mode; key mode from Credential Manager
- [ ] Stream parser → `AiEvent` with fixture tests
- [ ] ChatPane: markdown, code blocks with copy, tool-activity lines
- [ ] Follow-ups in the same conversation; Esc closes; new query = new chat
- [ ] Cancel in-flight response (Ctrl+.)
- [ ] **Spike D-007**: inline Approve / Deny for write/send/run tools
- [ ] Read-only default tool set; working directory = home folder
- [ ] Verify claude.ai connectors (Gmail, Calendar, Drive) work from the launcher
- [ ] Window grows into chat mode; shrinks back on new search

## M5 ChatGPT (Codex)
- [ ] Detect `codex` install + ChatGPT login
- [ ] Codex adapter (`codex exec --json`), parser + fixtures
- [ ] Strip / inject `OPENAI_API_KEY` by billing mode
- [ ] `g ` prefix; Shift+Tab switches provider; default provider setting
- [ ] Provider badge on the chat and the Ask row
- [ ] Approvals / sandbox mapping (per D-007)

## M6 Settings & integrations
- [ ] Settings window (glass, same tokens): hotkeys, Win-key override toggle, default provider, billing mode per provider, exclusions, startup
- [ ] Hotkey recorder control
- [ ] API keys stored in Credential Manager
- [ ] **MCP registry**: add/remove MCP servers once; writes Claude + Codex configs
- [ ] Status page: Everything / Claude / Codex health, index counts, log folder link
- [ ] Settings file in a synced folder (OneDrive) option

## M7 Polish & reliability
- [ ] Conversation history (SQLite) + `>history` to reopen
- [ ] Copy last answer, paste answer into the previous app
- [ ] Error toasts with "open log"
- [ ] Auto-update from GitHub Releases (Tauri updater)
- [ ] Multi-monitor and DPI-change handling
- [ ] Perf pass against budgets; idle memory check

## M8 Later horizon
Productivity
- [ ] Calculator + unit/currency conversion inline (`2^10`, `5 mi to km`)
- [ ] Clipboard history (text + images), pin, search
- [ ] Snippets / text expansion
- [ ] Quicklinks: `yt cats`, `gh repo`, custom web searches
- [ ] Emoji / symbol picker
- [ ] System commands: lock, sleep, restart, empty recycle bin, toggle dark mode
- [ ] Window management: snap left/right/halves/thirds, move to next monitor
- [ ] Process killer (`kill chrome`)
- [ ] Color picker

AI power features
- [ ] Ask about selected text (hotkey grabs selection, opens with it as context)
- [ ] Ask about a file (file result action: "Ask Claude about this")
- [ ] Screenshot region → ask AI
- [ ] Voice input (push-to-talk)
- [ ] Saved AI commands (prompt templates: "fix grammar", "summarize clipboard")
- [ ] Side-by-side: same prompt to Claude and ChatGPT

Integrations without AI (needs Google OAuth, D-008)
- [ ] `cal`: next events, join meeting link
- [ ] `mail`: unread count, open thread
- [ ] Drive file search merged into results
- [ ] Media controls (play/pause/next, now playing)

Extensibility
- [ ] Script commands: drop a `.ps1`/`.py` with a header comment into a folder → becomes a command (Raycast-style)
- [ ] Theme variants (tint color, blur strength)
- [ ] Per-machine overrides on top of synced settings
