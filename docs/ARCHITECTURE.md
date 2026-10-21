# Architecture

Goal: well-built, not enterprise. A single-user app for my own Windows
machines. Favor simple, readable code with clear module boundaries over
generality.

## Stack

| Layer | Choice |
|---|---|
| App shell | Tauri 2 |
| Backend | Rust (stable), `tokio` async runtime |
| UI | Svelte 5 + TypeScript + Vite |
| Styling | Plain CSS with design tokens (custom properties); no CSS framework |
| Window effect | `window-vibrancy` Acrylic (Win11 system backdrop; blur-behind on Win10); opaque surface if unavailable |
| Hotkeys | `tauri-plugin-global-shortcut` (Ctrl+Space) + a `WH_KEYBOARD_LL` hook via `windows-sys` (Windows key tap) |
| Tray, single instance, autostart | Tauri tray + `tauri-plugin-single-instance` + `tauri-plugin-autostart` |
| Fuzzy matching | `nucleo-matcher` |
| Storage | SQLite via `rusqlite` (history, frecency); TOML for settings |
| Secrets | Windows Credential Manager via `keyring` |
| IPC typing | `tauri-specta` generates TypeScript bindings from Rust types |
| Logging | `tracing` + `tracing-appender` (daily rolling file) |

## Processes

```
┌──────────────────────── desktop-assistant.exe ────────────────────────┐
│                                                                       │
│  Rust core (tokio)                         WebView2 (Svelte UI)       │
│  ┌──────────────┐  typed commands/events  ┌───────────────────────┐   │
│  │ ipc          │◄───────────────────────►│ SearchBar / Results   │   │
│  ├──────────────┤                         │ ChatPane / Approvals  │   │
│  │ hotkey       │  (LL keyboard hook thr) │ Settings              │   │
│  │ search       │                         └───────────────────────┘   │
│  │ ai           │                                                     │
│  │ settings     │                                                     │
│  │ store        │                                                     │
│  └──────┬───────┘                                                     │
└─────────┼─────────────────────────────────────────────────────────────┘
          │
          ├── Everything, if running (IPC via the Everything SDK, compiled in)
          ├── claude.exe   (child process, stream-json over stdio)
          └── codex.exe    (child process, JSON events over stdio)
```

- One window, created hidden at startup and never destroyed. Showing it is
  position + `show()` + focus, which keeps hotkey-to-visible under 50 ms.
  Close (Alt+F4) is turned into hide.
- It opens on the monitor under the mouse, centered, with its top edge 22%
  down the work area so it grows downward.
- Two heights only: search bar alone (empty query) or bar + a fixed-height
  results area + footer. The UI measures itself and calls
  `launcher_set_height`; resizing only on that switch avoids Acrylic redraw
  flicker on every keystroke.
- It hides when it loses focus, on Esc (the first Esc clears the query), or
  after running something. A toggle within 250 ms of a hide is ignored, so a
  hotkey that briefly steals focus closes the window instead of reopening it.
- Opened by: Ctrl+Space, tapping the Windows key, tray left-click, or
  launching the app again (single-instance hands off to the running copy).
- The tray menu has Open Launcher, Settings, Open Log Folder and Quit; menu
  items and result rows share `actions.rs`.
- Set `DA_KEEP_OPEN=1` to stop hide-on-blur while debugging (devtools).

## Source layout

Modules marked *(Mn)* don't exist yet; they're created in that milestone.

```
desktop-assistant/
├── package.json                 # all JS deps + scripts (dev, build, check, test, bindings)
├── src-tauri/
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── capabilities/            # Tauri permission sets per window
│   ├── icons/                   # app-icon.png is the source; rest generated
│   └── src/
│       ├── main.rs              # entry; calls lib::run()
│       ├── lib.rs               # builds the Tauri app, wires modules
│       ├── bin/gen-bindings.rs  # writes ui/src/lib/bindings.ts
│       ├── error.rs             # AppError (thiserror) + IpcError for the UI
│       ├── paths.rs             # config / data / log folders
│       ├── logging.rs           # tracing to rolling file (+ stderr in debug)
│       ├── state.rs             # AppState managed by Tauri
│       ├── actions.rs           # runs what the user picked (rows, tray)
│       ├── ipc/                 # #[tauri::command]s + specta builder; thin
│       ├── settings/            # Settings (layered over defaults), load/save/migrate
│       ├── search/
│       │   ├── mod.rs           # SearchProvider trait, Aggregator
│       │   ├── item.rs          # ResultItem, ItemKind, Action
│       │   ├── commands.rs      # built-in commands (settings, logs, reindex, quit)
│       │   ├── rank.rs          # final order: fuzzy + name bonuses + frecency
│       │   ├── apps.rs          # app index, provider, Start Menu watcher
│       │   └── files.rs         # file/folder results: Everything, else the built-in index
│       ├── ai/
│       │   ├── mod.rs           # AiEvent, AiError, ProviderId, BillingMode, Approvals
│       │   ├── chat.rs          # the current conversation: process, approvals, stop, idle shutdown
│       │   ├── claude/
│       │   │   ├── mod.rs       # find Claude Code, build its command line
│       │   │   └── protocol.rs  # stream-json lines → AiEvent (pure, fixture-tested)
│       │   └── codex.rs         # (M5) Codex CLI adapter
│       ├── window/              # show/hide/toggle, placement, glass, tray.rs
│       ├── hotkey/
│       │   ├── shortcut.rs      # Ctrl+Space (configurable) via global-shortcut
│       │   ├── tap.rs           # pure "lone Windows key tap" state machine
│       │   └── win_key.rs       # Windows-only LL keyboard hook thread
│       ├── desktop.rs           # desktop context for Claude: capture on open, prompt text
│       ├── file_index/          # built-in file index: scan, snapshot, watch, top-k search
│       ├── fuzzy.rs             # nucleo wrapper, scores normalized to 0..1
│       ├── icons.rs             # icon cache (memory + disk PNG) + `icon` protocol
│       ├── store/               # SQLite: frecency (later: history), migrations
│       ├── integrations/        # (M6) mcp_registry.rs
│       └── platform/            # `Shell`: apps, files, icons, open/reveal; stubs off Windows
│           └── win/             # COM on STA threads: AppsFolder, ShellExecuteEx, icons,
│                                #   everything.rs (SDK bindings), desktop.rs (foreground
│                                #   window, Explorer folder + selection)
├── src-tauri/third_party/everything-sdk/  # vendored Everything SDK C source (MIT)
├── ui/
│   ├── index.html
│   ├── vite.config.ts           # also the Vitest config
│   └── src/
│       ├── lib/bindings.ts      # GENERATED by tauri-specta; do not edit
│       ├── lib/latest.ts        # drops out-of-order responses
│       ├── lib/keys.ts          # key → intent for search and chat; footer hints
│       ├── lib/display.ts       # row text helpers (path shortening)
│       ├── lib/chat.ts          # AiEvent → transcript entries (pure)
│       ├── lib/markdown.ts      # Markdown → sanitized HTML (marked + DOMPurify)
│       ├── lib/stores/          # Svelte 5 rune classes (*.svelte.ts)
│       ├── components/          # SearchBar, ResultList, ChatPane, Footer, ...
│       ├── styles/tokens.css    # all colors, radii, blur, spacing
│       └── App.svelte
├── docs/
└── .github/workflows/           # test.yml (linux), build.yml (windows installer)
```

Rule: platform-specific code lives in `platform/win/` or behind
`#[cfg(windows)]` inside `hotkey/`. Everything else
compiles and tests on Linux, so CI and cloud sessions can run the test suite.

## Core types

```rust
// search/item.rs
pub struct ResultItem {
    pub id: String,          // stable: "app:<aumid>", "file:<path>", "cmd:<name>"
    pub kind: ItemKind,      // App | File | Folder | Command | AskAi
    pub title: String,
    pub subtitle: String,    // path, description
    pub icon: Option<String>,// key for the `icon` protocol; None = glyph by kind
    #[serde(skip)]
    pub score: f32,          // ordering only; never sent to the UI
    pub actions: Vec<Action>,// first = default (Enter)
}

// search/mod.rs
#[async_trait]
pub trait SearchProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError>;
}

// ai/mod.rs: what the UI hears about a conversation, in order.
// Sent as the `ai-stream` event: { conversation: u32, event: AiEvent }.
pub enum AiEvent {
    Started { model: String },
    TextDelta { text: String },
    ToolStarted { id: String, name: String, summary: String },
    ToolFinished { id: String, ok: bool },
    ApprovalRequested { id: String, tool: String, summary: String, detail: String },
    ApprovalResolved { id: String, allowed: bool },
    Done,
    Stopped,
    Error { message: String },
}

// ai/chat.rs: one conversation at a time.
impl Chat {
    async fn start(&self, text: &str, launch: &Launch, options: Options) -> Result<u32, AiError>;
    async fn follow_up(&self, text: &str, launch: &Launch, options: Options) -> Result<u32, AiError>;
    async fn respond(&self, request_id: &str, allow: bool) -> Result<(), AiError>;
    async fn stop(&self);   // interrupt the running turn
    async fn close(&self);  // end the conversation
}
// Codex (M5) gets its own adapter behind the same Chat and AiEvent.
```

## Query flow

1. UI sends `search_query(query, seq)` on every keystroke. `seq` increases; the UI
   drops responses older than the latest `seq`.
2. Prefix check first: `c ` forces Claude, `g ` forces ChatGPT, `>` lists
   commands. Forced AI skips search.
3. The aggregator runs all providers in parallel with a 40 ms budget,
   asking each for 4× the visible limit so frecency can lift weaker matches.
   What's in by then is shown. A provider that missed the budget keeps
   going (up to 3 s); when it finishes, the full re-ranked list goes to the
   UI as a `search-updated` event for that `seq`, applied only if it's still
   the newest search (D-020).
4. Results are merged, ranked (`rank.rs`), and capped at 8 visible rows. An
   "Ask Claude: <query>" row is always appended last.
5. Keys (`ui/src/lib/keys.ts`):
   - Enter runs the selected item's default action; Ctrl+1..9 runs row N.
   - Ctrl+Shift+Enter runs as administrator (desktop apps only).
   - Ctrl+Enter shows a file or folder in Explorer; Ctrl+Shift+C copies its
     path. The footer lists whichever of these the selected row supports.
   - If the only row is Ask AI, Enter sends to the default AI provider.
   - Tab always sends to AI. Shift+Tab switches provider. (M4/M5)
6. `item_activate(id, action)` → `actions.rs`: launch/run, record the open in
   `store` (frecency), hide the window.

### Apps

- `AppIndex` holds the Start menu's "All apps" (the AppsFolder) in memory:
  desktop programs and Store apps, each with a display name and a "target"
  (its AppsFolder parsing name: an AppUserModelID or a path). Uninstallers,
  readmes and web links are filtered out. The exe name becomes a keyword
  when it isn't already in the name ("procexp64").
- Indexed at startup, when either Start Menu folder changes (2 s debounce),
  when the launcher opens and the index is over 30 minutes old (catches
  Store installs), and on "Reindex Apps".
- Launch = `ShellExecuteEx` on the `shell:AppsFolder\<target>` item, the same
  path Start uses, with the `runas` verb for admin. Declining UAC isn't an
  error.
- All shell COM work runs on STA threads (`platform/win/sta.rs`): one
  long-lived worker for listing and icons, a fresh thread per launch.

### Files

Two sources, one provider (`search/files.rs`). Each file search asks
Everything first; if it isn't running (a near-instant failure), it uses the
built-in index. Nothing has to be installed. (D-017)

- Query: `f `/`d ` prefixes limit to files/folders. Queries under 2
  characters skip file search. Everything gets the text as-is (its syntax
  like `ext:pdf` works) plus one `!path:"\<folder>\"` per exclusion. The
  built-in index understands names, `ext:` and the prefixes, and ignores
  other Everything-only syntax.
- Scoring, both sources: name (fuzzy, ignoring `ext:`-style tokens) ×
  recency (halves every 30 days) × 0.8, so an equally good app wins. Up to
  100 candidates are re-ranked with frecency like everything else.
- Everything: the SDK is C with global state that runs each query on the
  calling thread, so all calls go through one worker thread. Each search
  bumps a generation counter; queued searches that are already out of date
  return immediately instead of piling up while typing.
- Actions: Open (ShellExecuteEx), Show in folder
  (`SHOpenFolderAndSelectItems`), Copy path (clipboard plugin).

#### Built-in file index (`file_index/`)

- Covers `search.index_folders` (default `~`, the user folder). Skips hidden
  and system items (that covers AppData), dot-folders, symlinks/junctions,
  and doesn't enter folders in `search.exclusions`. Caps at 2 million entries.
- Shape: folder path → its direct children (`BTreeMap`), names as boxed
  strings with kind and modified time. Search scores every entry under a
  read lock and keeps the top 100 in a min-heap, run with `spawn_blocking`.
  From 20k entries it splits the folders across up to 8 threads. Each
  search bumps a generation counter and checks it every 4k entries, so a
  search for a query you've typed past stops early.
- Lifecycle, on its own thread: load the snapshot (`file-index.txt` in the
  data folder) so search works instantly → start watching → full rescan to
  catch changes made while closed → apply changes. Snapshot saved after each
  scan and every 5 minutes if anything changed.
- First run (no snapshot): folders are added to the index as they're read
  (every 0.3 s), so search works during the scan. Later rescans build the
  new tree on the side and swap it in.
- Progress: `IndexStatus { scanning, incomplete, entries }`, from
  `file_index_status` and the `file-index-changed` event. While
  `incomplete`, the footer says "Indexing your files… N found so far".
- Watching: each root non-recursively, plus each indexed folder directly
  inside a root recursively, so AppData's churn never reaches us. Changes are
  batched (0.5 s quiet, 3 s max); each re-reads only the folder it happened
  in, then indexes new folders and drops deleted ones. Events in folders that
  aren't indexed are ignored. If the OS reports lost events, rescan
  everything (at most every 10 minutes).
- Started at launch only if Everything isn't running; otherwise the first
  search that finds Everything gone starts it.

### Icons

Result rows reference icons by key (FNV-1a hash of an identity). Apps key
by target; files share one icon per extension, except programs, shortcuts
and folders, which key by path. The UI
loads `convertFileSrc(key, "icon")`, served by the `icon` URI protocol from
memory → `%LOCALAPPDATA%\desktop-assistant\icons\<key>.png` → extracted from
the shell on demand (`IShellItemImageFactory`, 64 px). Failures are
remembered for the session; Reindex clears the cache. Keys are validated as
16 lowercase hex chars before touching the disk.

### Ranking

`(fuzzy + name_bonus) × (1 + frecency_boost)`, sorted best first.

- `fuzzy`: provider score in 0..1 (nucleo, normalized to the query matched
  against itself).
- `name_bonus`: +0.3 if the name starts with the query, +0.15 if any word does.
- `frecency_boost`: 0..1 on a log scale of the decayed open count (two-week
  half-life), maxing out around 20 recent opens.
- Typing a result's whole name always ranks it first.

Tune in `rank.rs` only; providers return raw fuzzy scores.

## AI flow

- **Finding Claude Code:** `ai/claude/` looks for `claude` on PATH, then the
  native installer's folder (`~/.local/bin`), then npm's
  (`%APPDATA%\npm`). An npm install is started as `node cli.js` rather than
  through its `.cmd` shim, so cmd.exe never touches the arguments.
  `ai.claude.path` overrides the search.
- **Install and sign-in check:** `claude auth status --json` runs in the
  background at startup, when the launcher opens (every 30 min while fine,
  every 15 s while broken) and after a chat error. While Claude Code is
  missing or signed out, the "Ask Claude" row becomes "Install Claude Code
  to ask: …" (opens the setup page) or "Sign in to Claude Code to ask: …"
  (runs `claude auth login` in a console window, then checks again). Tab
  runs the fix too. Both are also commands you can search for. WinGet's
  links folder is checked as well, since the launcher's PATH predates an
  install made after login.
- **One process per conversation:**
  `claude -p --input-format stream-json --output-format stream-json --verbose
  --include-partial-messages --permission-prompt-tool stdio --model <model>`,
  with the working folder set to Explorer's folder (else your home folder)
  and a short system prompt
  appended (it's in a popup, keep answers short, act on files when asked).
  Never `--bare`, which ignores the subscription login. The process stays up
  between turns, so follow-ups are written to stdin and start instantly.
  After 10 minutes idle it is shut down; the next follow-up starts a new one
  with `--resume <session id>` in the conversation's original folder (Claude
  Code keeps sessions per folder), so the conversation carries on. A new
  question kills the old process.
- **Desktop context (D-021):** as the launcher opens, before it takes focus,
  `desktop.rs` records the window in front (title, program). For File
  Explorer or the desktop it then asks the shell, in the background, for the
  folder shown and the selected files (`IShellWindows` → the window's
  `IShellBrowser` → `IFolderView`; with Windows 11 tabs, the visible tab).
  Asking waits up to 0.8 s for that. The context goes in front of the message
  as a `<desktop_context>` block (a follow-up repeats it only if it changed),
  Explorer's folder becomes the new conversation's working folder, and the
  chat shows "Sees: Downloads · 2 selected" under your message.
  `ai.claude.desktop_context = false` turns it off.
- **Model:** `ai.claude.model`, "sonnet" (Sonnet 5.5) by default, or "haiku"
  (Haiku 5.5) for quick questions. Any alias or full model id works.
- **Billing mode:** in subscription mode the child env drops
  `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` (otherwise Claude Code
  silently prefers the key). API key mode currently uses the
  `ANTHROPIC_API_KEY` environment variable; the settings window (M6) moves it
  to Credential Manager. Same pattern for Codex with `OPENAI_API_KEY`.
- **Connectors:** Claude gets claude.ai connectors automatically from its
  login. Local MCP servers come from `mcp_registry`, which writes the same list
  into Claude's and Codex's config files so I only add a service once.
- **Approvals (D-018):** with `--permission-prompt-tool stdio`, Claude Code
  sends a `can_use_tool` control request on stdout whenever its rules say a
  tool needs permission (reading files and harmless commands like `ls` don't;
  writing, moving, deleting and most commands do). It becomes
  `ApprovalRequested`; the popup shows the full command or file content with
  Approve (Ctrl+Y) and Deny (Ctrl+N or Esc), and the answer goes back as a
  `control_response`. `ai.claude.approvals = "auto"` adds
  `--permission-mode auto`: Claude Code's own classifier approves routine
  actions and only asks when unsure.
- **Stopping:** Esc or Ctrl+. sends an `interrupt` control request. The turn
  ends with an error result, which `chat.rs` reports as `Stopped`.
- **Parsing:** `protocol.rs` turns each stdout line into `AiEvent`s. It's a
  pure function, tested against sanitized recordings of real sessions in
  `src-tauri/tests/fixtures/claude/`. Subagent output is skipped; crashes
  surface with the last lines of stderr.
- **UI:** Tab (or Enter on the "Ask Claude" row, or a `c ` prefix + Enter)
  starts a chat. The search box becomes the follow-up box, the results area
  becomes `ChatPane` (Markdown rendered then sanitized; links open in the
  browser, code blocks get a Copy button). Answers can't load anything:
  images show as links, forms are stripped, and the CSP blocks remote
  content and form posts, so text Claude read from a file can't smuggle data
  out through the page. The UI holds early `ai-stream`
  events until `ai_ask` returns the conversation id, so a late event from
  an old conversation can't leak in. Esc backs out one step at a time: deny,
  stop, clear the draft, close the chat. A finished chat is put away when the
  launcher reopens after 5 minutes hidden.
- **Screenshots (planned, D-019):** opt-in only, never sent automatically,
  and the launcher itself must never be in the picture: capture the window
  you were using by its handle, and exclude the launcher from capture while
  a full-screen shot is taken.

## Windows key override

`hotkey/win_key.rs` installs a `WH_KEYBOARD_LL` hook on a dedicated thread with
its own message loop. The decision logic lives in `hotkey/tap.rs` as a pure
state machine with unit tests; the hook just feeds it events.

- Win down: let it through and remember it. With Shift/Ctrl/Alt already held,
  it's a combo from the start.
- Any other key down while Win is held: a combo (Win+E, Win+V...), so no tap.
- Win up, for a Win down we saw (tap or combo alike): swallow the real key-up
  and inject, in one `SendInput`, mask down, mask up, Win up. Windows sees
  Win+mask, a combo, so Start never opens. The mask is `0xE8`, an unassigned
  virtual key (AutoHotkey's trick). Masking every release, not just taps,
  means a misjudged press can't let Start through either. (D-015)
- Win up after a lone tap also posts "toggle launcher" to the main thread.
- If `SendInput` doesn't insert all three events, the real key-up goes
  through instead, so Windows can never think Win is stuck down.
- Injected events carry a tag in `dwExtraInfo` so the hook skips its own.
- The hook callback only updates state, injects and posts. Windows silently
  removes hooks that take too long (`LowLevelHooksTimeout`).
- Limits: it can't see keys while an elevated (admin) window has focus unless
  this app also runs elevated. Ctrl+Esc still opens Start.
  `hotkeys.win_key_override = false` turns it off (applied at startup).

## Storage & paths

| What | Where |
|---|---|
| Settings | `%APPDATA%\desktop-assistant\settings.toml` |
| History, frecency | `%LOCALAPPDATA%\desktop-assistant\data.db` |
| Logs | `%LOCALAPPDATA%\desktop-assistant\logs\` (7 days kept) |
| API keys | Windows Credential Manager, service `desktop-assistant` |

Settings has a `version` field; loading an older file runs a small migration
function and rewrites it. The file on disk is layered over a full table of
defaults before parsing, so missing fields get defaults while the Rust types
(and generated TypeScript types) stay non-optional. A file that fails to parse
is moved to `settings.toml.bad` and replaced with defaults.

## Performance budgets

| Path | Budget |
|---|---|
| Hotkey to visible, focused window | < 50 ms |
| Keystroke to results rendered | < 50 ms (p95) |
| Everything query | < 30 ms |
| Ask AI to first streamed token (warm) | < 2 s |
| Idle memory | < 100 MB |
| Idle CPU | ~0% |

Blow a budget means fix it before adding features on top.

## UI / theme

Liquid glass:
- Window: frameless and transparent with Acrylic behind it. Windows 11 rounds
  the corners (8 px) and draws the outer shadow (D-009).
- On top: a low-alpha tint (`--surface`), a diagonal sheen, and a 1 px inner
  highlight bright at the top edge and faint at the bottom.
- When Acrylic isn't available (Win10 without it, Linux dev), `--surface` is
  opaque. `main.ts` sets `<html data-glass="on">` from `app_info().glass`.
- Selected row: a brighter glass "pill" with a subtle gradient sheen.
- Follows the Windows light/dark setting. All values live in
  `ui/src/styles/tokens.css`; components never hard-code colors.
- Keyboard-first: everything works without a mouse. Animations ≤ 150 ms and
  respect "reduce motion".
