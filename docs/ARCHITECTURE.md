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
│       ├── mcp/                 # the launcher's MCP server for Claude: HTTP + token,
│       │                        #   JSON-RPC (protocol.rs), the tools (tools.rs)
│       ├── state.rs             # AppState managed by Tauri
│       ├── actions.rs           # runs what the user picked (rows, tray); copy, paste, open link
│       ├── toast.rs             # short footer messages from the backend (errors offer the log)
│       ├── features/            # one folder per feature that plugs into search (D-027):
│       │                        #   calc, quicklinks, emoji, clipboard, snippets, scripts,
│       │                        #   updater, system, window_layout, processes, media, color;
│       │                        #   each has provider(), start(), its settings struct
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
│       ├── lib/glyphs.ts        # built-in result symbols (`glyph:<name>` icons)
│       ├── lib/latest.ts        # drops out-of-order responses
│       ├── lib/keys.ts          # key → intent for search and chat; footer hints
│       ├── lib/display.ts       # row text helpers (path shortening)
│       ├── lib/chat.ts          # AiEvent → transcript entries (pure)
│       ├── lib/markdown.ts      # Markdown → sanitized HTML (marked + DOMPurify)
│       ├── lib/stores/          # Svelte 5 rune classes (*.svelte.ts)
│       ├── components/          # SearchBar, ResultList, ChatPane, Footer, ...
│       │   └── settings/        # settings menu: shared controls + one *Section per area
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
// search/item.rs (build with ResultItem::new(id, kind, title).with_…)
pub struct ResultItem {
    pub id: String,          // stable: "app:<aumid>", "file:<path>", "cmd:<name>"
    pub kind: ItemKind,      // App | File | Folder | Command | AskAi | Answer | Clip | ...
    pub title: String,
    pub subtitle: String,    // path, description
    pub icon: Option<String>,// icon protocol key, "thumb/<name>", "glyph:<name>",
                             //   "text:<emoji>", "color:#rrggbb"; None = glyph by kind
    #[serde(skip)]
    pub score: f32,          // ordering only; never sent to the UI
    pub actions: Vec<Action>,// first = default (Enter)
    pub accessory: Option<String>, // right-hand text ("Pinned")
    pub confirm: Option<String>,   // ask first: Enter again within 4 s runs it
}

// search/mod.rs
#[async_trait]
pub trait SearchProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError>;
    fn claims(&self, query: &str) -> bool { false }   // "kill …": only claimers run
    fn owns(&self, id: &str) -> bool { false }        // its ids' activations come here
    async fn activate(&self, id: &str, action: Action) -> Result<Activation, SearchError>;
}

// What follows an activation. Rust does Hide/Copy/Paste/OpenUrl (actions.rs);
// the UI does SetQuery/Ask/Notice/Refresh/OpenSettings.
pub enum Activation {
    Hide, Copy { text }, Paste { text: Option<String> }, OpenUrl { url },
    SetQuery { query }, Ask { prompt, provider }, Notice { message }, Refresh, OpenSettings,
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
   - Other actions have fixed keys, used by whichever rows offer them:
     Ctrl+Enter also pastes (clips, answers), Ctrl+Shift+C also copies,
     Ctrl+Shift+P pins, Ctrl+D deletes, Ctrl+Shift+A asks the AI about the
     row. Enter's footer label follows the row's default action.
   - A row with `confirm` (restart, shut down, end a process) shows its
     question in the footer; Enter again within 4 s runs it.
6. `item_activate(id, action)` → `actions.rs`: the provider that `owns` the
   id runs it (else the built-in apps/files/commands code), the open is
   recorded in `store` (frecency), and the returned `Activation` is carried
   out: hide, copy, paste into the window you came from (hide, refocus it,
   Ctrl+V), or open a link. The UI gets the `Activation` back and does the
   rest (new query, ask the AI, a footer notice, refresh, settings).

### Features

Everything added after the launcher's core (calculator, clipboard, system
commands, ...) lives in `features/<name>/` (D-027): `provider(ctx)` returns
its `SearchProvider` (with `claims`/`owns`/`activate`), `start(app)` its
background work, and the folder holds its settings struct (a section of
`Settings`) and any Windows code (`#[cfg(windows)] mod win;`). The
`ProviderContext` gives it the shell, icons, store, shared settings (always
current), the desktop tracker (the window you came from) and paths. Its
settings UI is `ui/src/components/settings/<Name>Section.svelte`, built
from the shared controls (Row, Toggle, Choice, TextField, ChipList, ...).
Shared helpers: `Shell::focus_window` / `press_keys` (Windows), `actions::paste`
and `copy_text`, `toast::info` / `toast::error`, thumbnails served from
`AppPaths::thumbnails_dir()` as `thumb/<name>` icons.

#### Calculator, quicklinks, emoji (`features/calc`, `features/quicklinks`, `features/emoji`)

- **Calculator.** fend-core evaluates; `parse.rs` decides first whether a query is a calculation, because the answer row always ranks first and a false positive steals Enter from the app you meant. A query needs a digit and an operation (an operator, a `to`/`in`/`as` conversion, a function, `mod`, `of`). Dates, versions and phone numbers (`2024-01-01`, `1.2.3`, `555-1234`), hyphenated names (`k-9`), one-letter prefixes (`c `, `g `) and anything over 100 characters or 12 brackets deep are left alone. A leading `=` forces an answer. `rewrite.rs` turns everyday notation into fend's: `€50` → `50 EUR`, `72 f to c` → `72 °F to °C` (only when both sides are temperatures), `in` after a number → inch, `mb` → `MB`, `3x4` → `3 × 4`, and desk-calculator percentages (`200 + 10%` = 220, `80 * 15%` = 12). fend runs on a blocking thread with a 25 ms deadline. `format.rs` marks approximate answers with `≈`, rounds approximate conversions to 4 decimals and money to cents, groups digits in the title (Enter copies the plain value) and prefixes hex/binary/octal answers. Enter copies, Ctrl+Enter pastes into the window you came from.
- **Exchange rates.** The European Central Bank's daily reference rates (`eurofxref-daily.xml`, about 30 currencies, euro base, no key), downloaded with WinHTTP by a task started in `calc::start`. It downloads when the saved rates are over a day old, checks hourly (10 minutes after a failure), and saves to `%LOCALAPPDATA%\desktop-assistant\exchange-rates.json` so conversions work offline. Search only reads what's in memory. Without rates, a currency query shows "Exchange rates aren't downloaded yet" (Enter retries); a currency the ECB doesn't publish shows "No exchange rate for XXX".
- **Quicklinks.** `quicklinks.links` is a list of keyword, name and an http(s) address with `{query}`. `yt lofi` shows "Search YouTube for “lofi”" above apps and opens the address with the text percent-encoded. The keyword alone shows "Search YouTube · Type to search" below a good app match, and Enter fills in `yt `. A link without `{query}` just opens. Defaults: g, yt, gh, maps, wiki, amazon, translate, ddg.
- **Emoji.** `:` or `emoji ` claims the query. The index (emojis crate, Unicode ≤ 15.1 since newer emoji are empty boxes on Windows, plus about 120 text symbols: arrows, checks, math, currency, typography, keys) is built once in `provider()`. It matches names, GitHub shortcodes, extra keywords (`lol`, `->`, `!=`) and emoticons (`:)`). `:` alone lists recently picked emoji (frecency), then popular ones. Enter pastes into the window you came from, Ctrl+Shift+C copies, and `emoji.skin_tone` applies to icons and paste. Without a prefix, a query that is exactly an emoji's name or shortcode (3+ characters: "thumbs up", "degree") gets one row at score 0.05, and the ranker gives emoji no exact-name tier, so "fire" never outranks Firefox.

#### Clipboard history (`features/clipboard/`)

- **Listening (Windows, `win.rs`):** a thread owns a message-only window registered with `AddClipboardFormatListener`. On each `WM_CLIPBOARDUPDATE` it skips:
  - our own writes: the change's sequence number equals the last one we wrote; it waits for a write still in progress first;
  - copies an app marks private: `ExcludeClipboardContentFromMonitorProcessing`, `Clipboard Viewer Ignore`, `CanIncludeInClipboardHistory` = 0;
  - copies whose program is on `clipboard.ignore_apps` (password managers by default). The program is the clipboard owner, else the window in front.

  Otherwise it copies the raw bytes out and closes the clipboard at once, because an app that copies twice fails while it's held open. Order: files (`CF_HDROP`), then text (`CF_UNICODETEXT`), then images (`PNG`, `CF_DIBV5`, `CF_DIB`). A recorder thread decodes and stores the copy. The PNG/DIB/file-list parsers are pure and tested on Linux.
- **Storage:** its own `clipboard.db` in the data folder. Images are kept as `clipboard\<hash>.png`, with a 64 px thumbnail at `thumbnails\clip-<hash>.png` shown as `thumb/clip-<hash>`.
  - Copying something already listed moves it to the top. Images are compared by pixels, so the same picture encoded differently is still a repeat.
  - `max_items` (200) counts unpinned entries only; image files are deleted with their rows; leftover files are removed at startup.
  - The list is mirrored in memory (the first 1000 characters of each text), so `query` never reads the disk.
- **Search:** `clip` or `clipboard` (alone or followed by text) claims the query.
  - Order: pinned first, then newest. `rank.rs` ranks Clip rows by their provider score only, so name matches and frecency don't reorder history.
  - Text after the prefix filters by fuzzy match on the clip text or its program, with a minimum score, because long texts contain most letters in order somewhere.
  - Keys: Enter pastes, Ctrl+Shift+C copies, Ctrl+Shift+P pins, Ctrl+D deletes. To paste, the entry goes back on the clipboard (text, PNG + bitmap, or a file list, written through `arboard`) and `Activation::Paste` does the rest.
  - "Clear Clipboard History" asks first and keeps pinned entries. "Clipboard History" in normal search opens `clip `.
- `system.rs` reads and writes the clipboard for both clipboard history and snippets.

#### Snippets (`features/snippets/`)

- **Settings:** `snippets.items` ({name, keyword, text}).
- **Search:**
  - A snippet is found by its keyword (typed with or without the trigger) or by its name.
  - `snippet`, `snippets` and `snip <text>` claim the query and list them. `snip` alone lists them without claiming, so Snipping Tool still shows.
  - Enter pastes, Ctrl+Shift+C copies.
- **Placeholders:** `{clipboard}`, `{date}` (2026-10-21), `{time}` (14:05), `{datetime}`. Anything else in braces is left as typed.
- **Expand as you type** (`snippets.expand_as_you_type`, off by default):
  - A task checks the settings once a second and starts or stops `hook.rs`.
  - Low-level keyboard and mouse hooks on a time-critical thread only queue key-downs (with modifier state and the window in front) and clicks to a worker, so typing never waits. Keys tagged `INJECTED_TAG` are skipped.
  - The worker translates keys with the front window's layout (`ToUnicodeEx`, leaving dead-key state alone). `typing.rs` (pure) keeps the last 64 characters and resets on clicks, window changes, Ctrl/Alt/Win shortcuts and keys that move the caret.
  - When the text typed ends with a keyword that starts with `snippets.trigger` (";"), it:
    1. waits for modifiers to be released;
    2. sends Backspace for each character of the keyword;
    3. writes the snippet to the clipboard, marked private;
    4. sends Ctrl+V;
    5. after 750 ms puts the old clipboard content back, unless something new was copied meanwhile.
  - Typing in the launcher is ignored. The hooks are re-installed every 10 s (D-025).

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
  native installer's folder (`~/.local/bin`), npm's (`%APPDATA%\npm`) and
  WinGet's links folder. For an npm `.cmd` shim it reads the shim to find
  what it runs (a native `claude.exe`, or `cli.js` under node) and starts
  that directly, so cmd.exe never touches the arguments.
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
- **Warm start (D-024):** when the query starts to look like a question for
  Claude (`c ` prefix, three or more words, or only the "Ask" row matching),
  the UI calls `ai_warm`, which starts a spare process with the options the
  next question would use (same folder, model, tools). `ai_ask` claims it if
  they still match; its early output is held until then. Unclaimed, it's
  closed after 2 minutes. Measured: first words ~1.3 s after asking instead
  of ~2.5 s. `ai.claude.warm_start = false` turns it off.
- **Model:** `ai.claude.model`, "sonnet" (Sonnet 5.5) by default, or "haiku"
  (Haiku 5.5) for quick questions. Any alias or full model id works.
- **Billing mode:** in subscription mode the child env drops
  `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` (otherwise Claude Code
  silently prefers the key). API key mode currently uses the
  `ANTHROPIC_API_KEY` environment variable; the settings window (M6) moves it
  to Credential Manager. Same pattern for Codex with `OPENAI_API_KEY`.
- **Connectors and skills:** Claude gets claude.ai connectors and skills
  automatically from its login. Claude Code (2.1.273+) syncs the account's
  skills into `~/.claude/skills/synced/` in the background, and also loads
  hand-written ones from `~/.claude/skills/`. Both need the claude.ai
  sign-in: API key mode gets neither. A skill added on claude.ai shows up
  once that sync has run (usually by the next question). Local MCP servers come from `mcp_registry`, which writes the same list
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
- **Launcher tools (MCP, D-023):** at startup the launcher serves MCP over
  HTTP on 127.0.0.1 (random port, secret token per run; requests with an
  `Origin` or another host name are refused) and writes `mcp-config.json`
  and `claude-settings.json` to its data folder. Each conversation gets
  `--mcp-config=` and `--settings=` pointing at them; the user's own MCP
  servers and claude.ai connectors still load. Tools, seen by Claude as
  `mcp__launcher__*`:
  - `get_desktop_context`: the desktop context, Explorer re-read fresh
  - `search`: the launcher's app and file search (waits for slow sources)
  - `open`, `reveal`: open a path or app id; show a path in Explorer
  - `screenshot`: the window you were in, by handle with `PrintWindow`, so
    the launcher is never in it (D-019); scaled to 1568 px, sent as PNG
  The settings file pre-approves the two reading tools and forces an
  approval for `screenshot`, even in auto mode; `open` and `reveal` follow
  the normal rules.

## Windows key override

`hotkey/win_key.rs` installs a `WH_KEYBOARD_LL` hook on a dedicated
time-critical thread with its own message loop. The decision logic lives in
`hotkey/tap.rs` as a pure state machine with unit tests; the hook just feeds
it events and carries out its verdicts. (D-022)

- Win down: hidden from Windows. With Shift/Ctrl/Alt already held (or the
  other Win key down), it's a shortcut from the start and goes through.
- Another key down while a hidden Win is held: a shortcut (Win+E,
  Win+Shift+S...). Win is injected for Windows first, then the key goes
  through; keys injected inside a hook are handled before the hooked key, so
  the order is right. From then on Win behaves normally.
- Win+L with Win hidden: the hook calls `LockWorkStation` itself (Windows
  ignores an injected Win for locking) and hides the L.
- Win up after a hidden press: hidden too, so Windows never saw the key and
  Start can't open. If nothing came in between, it was a lone tap: "toggle
  launcher" is posted to the main thread.
- Win up that Windows saw go down (a shortcut, or a press the hook missed
  because Windows skipped it): the real key-up is swallowed and replayed as
  mask down, mask up, Win up in one `SendInput`. Windows sees Win+mask, a
  shortcut, so Start stays closed. The mask is `0xE8`, an unassigned virtual
  key (AutoHotkey's trick). If `SendInput` fails, the real key-up goes
  through, so Windows can never think Win is stuck down. (D-015)
- Injected events carry a tag in `dwExtraInfo` so the hook skips its own.
- The hook callback only updates state, injects and posts, and its thread
  runs at time-critical priority: Windows skips a hook that answers late
  (`LowLevelHooksTimeout`), which is how a tap used to reach Start while the
  app was busy. Parallel file search leaves a core free for the same reason.
- Windows also removes a late hook for good, silently, and calls the newest
  hook first. So a timer on the hook thread re-installs it every 5 s while no
  Windows key is held: it comes back if removed, and stays ahead of hooks
  other apps install later (D-025). Settings shows whether it's running and
  when it last saw the key.
- Limits: it can't see keys while an elevated (admin) window has focus unless
  this app also runs elevated. Ctrl+Esc and the Start button still open
  Start. `hotkeys.win_key_override = false` turns it off (applied at startup).

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

## Settings menu

Opened from the launcher (the "Settings" command, Ctrl+, in search, or the
tray's Settings): `SettingsPane` replaces the results area, the way the chat
does. Every change is saved at once through `settings_set`, which applies
what it can live: the shortcut (re-registered; if the new one can't be,
nothing changes and the menu says why), the Windows key hook (started or
stopped), start at login. Claude and search options are read fresh on each
use, so they apply at once too. Index folders and exclusions shape the
running index, so the menu offers a restart for them (`app_restart` closes
Claude Code first). The shortcut recorder pauses the current shortcut while
it listens. `app_status` feeds the status lines: Claude Code's sign-in, the
index or Everything, and the Windows key hook. "Open Settings File" is still
there for hand edits (read at startup).

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
