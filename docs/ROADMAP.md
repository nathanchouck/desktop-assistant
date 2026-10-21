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

## M1 Shell: window, hotkeys, glass ✅ (pending on-device check)
- [x] Frameless, transparent, always-on-top window created hidden at startup
- [x] Center on the monitor with the cursor; two heights (bar only / bar + results)
- [x] Acrylic via `window-vibrancy`; opaque fallback when unavailable
- [x] Glass design tokens + SearchBar + ResultList + footer with key hints
- [x] Ctrl+Space global shortcut toggles the window
- [x] **Windows-key tap override** (`WH_KEYBOARD_LL`, combos pass through, masked replay of every Win release keeps Start closed; D-015)
- [x] Hide on blur, on Esc (first Esc clears), and after launching
- [x] Tray icon: Open, Settings, Open Log Folder, Quit (Reindex moves to M2)
- [x] Single-instance guard (second launch shows the running launcher)
- [x] Start with Windows (`general.launch_at_login`, applied at startup in release builds)
- [x] Light/dark follows Windows; reduce-motion respected
- [ ] Measure hotkey-to-visible (< 50 ms): logged at debug level; needs a number from a real machine

Check on a real Windows machine:
- [x] Glass looks right in light and dark mode (tune `--surface` in `tokens.css`)
- [x] Windows key tap opens the launcher
- [ ] Start/search stays closed every time (D-012 broke it; fixed per D-015, re-check)
- [x] Win+E, Win+V, Win+Shift+S, Win+L still work
- [x] Launcher takes keyboard focus when opened from the Windows key and Ctrl+Space
- [x] Ctrl+Space doesn't clash with your keyboard layout's input switching

## M2 App launcher ✅ (pending on-device check)
- [x] Tray menu + built-in command: Reindex Apps
- [x] Index the Start menu's "All apps" (`shell:AppsFolder`): desktop programs and Store apps
- [x] Icon extraction + on-disk icon cache, served via an `icon` protocol
- [x] `nucleo` fuzzy matching, acronyms ("vsc" → Visual Studio Code), exe-name keywords
- [x] SQLite store + frecency recording
- [x] Ranker: fuzzy + name-prefix bonus × frecency; exact name always first
- [x] Launch via ShellExecuteEx; Ctrl+Shift+Enter runs as administrator
- [x] Re-index when Start Menu folders change (watcher), and when stale on open
- [x] Keyboard nav: ↑/↓, Enter, Ctrl+1..9, Esc

Check on a real Windows machine:
- [ ] Your apps all show up (and no junk like uninstallers)
- [ ] Icons look sharp in light and dark mode
- [ ] Launched apps come to the front
- [ ] Ctrl+Shift+Enter shows the UAC prompt for desktop apps
- [ ] Apps you open often move up within a day of use

## M3 File search ✅ (pending on-device check)
- [x] Built-in file index of your folders (default: user folder), saved between runs, kept current by watching (D-017)
- [x] Everything used automatically when running (covers every drive); SDK compiled in, no DLL
- [x] ~~Fix-it row when Everything is missing~~ (dropped: Everything is optional now)
- [x] Merge file/folder results with apps; files rank below equally good app matches
- [x] Exclusions (`node_modules`, `.git`, `AppData` temp, `$Recycle.Bin`) in settings
- [x] File actions: Open, Show in folder (Ctrl+Enter), Copy path (Ctrl+Shift+C)
- [ ] File actions: Open with…, Delete to Recycle Bin (needs an action menu; M7)
- [x] Filters: `f ` files only, `d ` folders only, `ext:pdf` (plus any Everything syntax when it's running)
- [x] Query-sequence handling so stale results never flash (UI) and stale searches skip (worker)
- [x] Windows CI installs Everything and runs a real search test
- [x] Big indexes: results that miss the 40 ms budget arrive a moment later instead of being dropped; index search runs in parallel and stops when you type past it (D-020)
- [x] Footer shows indexing progress until the first scan is done; files show up during the scan

Check on a real Windows machine:
- [ ] Without Everything: files in your user folder show up as you type (first launch: the footer shows the scan's progress)
- [ ] A file you just saved or renamed shows up within a few seconds
- [ ] With Everything running: files anywhere on your drives show up
- [ ] Ctrl+Enter opens Explorer with the file selected; Ctrl+Shift+C copies the path
- [ ] Nothing from AppData, `node_modules`, `.git` or Temp shows up

## M4 Claude
Goal: Claude handles the tedious stuff ("move this file into my Astro folder",
"sort Downloads by type"), aware of what's on screen.

- [x] Detect `claude` install + login status; fix-it row if missing (install page / sign-in window)
- [x] Model setting: Sonnet 5.5 by default, Haiku 5.5 as an option (e.g. for quick questions); `--model` per run
- [x] Desktop context captured when the launcher opens: the window you were in (title, app), and for File Explorer its open folder and selected files (Shell COM: `IShellWindows`) (D-021)
- [x] Desktop MCP server hosted by the launcher, so Claude can ask for context and act on demand: active window, Explorer folder/selection, search files/apps (our index), reveal/open (D-023)
- [x] Opt-in screenshot of the active window for "what am I looking at" questions (vision), never sent automatically (always asks, even in auto mode); the launcher is never in the shot (D-019)
- [x] Working directory = the open Explorer folder when there is one
- [x] Routing: no matches + Enter → AI; Tab → AI; `c ` prefix; "Ask Claude" row always last
- [x] Long-lived stream-json child process (resumes the session after an idle shutdown)
- [ ] Warm spare process so the first question skips startup
- [x] Strip `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` in subscription mode
- [ ] Key mode from Credential Manager (with the M6 settings window; env var for now)
- [x] Stream parser → `AiEvent` with fixture tests
- [x] ChatPane: markdown, code blocks with copy, tool-activity lines
- [x] Follow-ups in the same conversation; Esc closes; new query = new chat
- [x] Cancel in-flight response (Ctrl+. or Esc)
- [x] **Spike D-007**: inline Approve / Deny for write/send/run tools (D-018)
- [x] Moving/renaming/deleting files asks first (Approve / Deny in the popup); `approvals = "auto"` hands routine ones to Claude Code's auto mode
- [ ] Verify claude.ai connectors (Gmail, Calendar, Drive) work from the launcher
- [x] Window grows into chat mode; shrinks back on new search

## M5 ChatGPT (Codex)
- [ ] Detect `codex` install + ChatGPT login
- [ ] Codex adapter (`codex exec --json`), parser + fixtures
- [ ] Strip / inject `OPENAI_API_KEY` by billing mode
- [ ] `g ` prefix; Shift+Tab switches provider; default provider setting
- [ ] Provider badge on the chat and the Ask row
- [ ] Approvals / sandbox mapping (per D-007)

## M6 Settings & integrations
- [ ] Settings window (glass, same tokens): hotkeys, Win-key override toggle, default provider, billing mode per provider, exclusions, startup
- [ ] Apply settings live (today hotkeys and file exclusions are read at startup)
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
