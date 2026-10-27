# Decisions

Short records of choices and why. Add a new entry rather than editing an old
one when a decision changes.

## D-001 Tauri 2 over Electron or native WinUI
Small binary (~10 MB vs ~150 MB), low idle memory, Rust for the Win32 and
search work, and web tech for the glass UI, which is much faster to iterate on
than WinUI XAML. Cost: Rust plus TS, two languages.

## D-002 Svelte 5 for the UI
Small runtime and little boilerplate for a UI with a handful of components.
React would also be fine; it's not worth revisiting.

## D-003 Everything for file search (required)
Windows Search is slow and misses files. Everything reads the NTFS file table
and answers in milliseconds. Building our own file-table reader needs admin
rights and is a lot of fragile code for no gain. The app checks on startup
and shows a "Install / start Everything" row if it's missing.

## D-004 AI through the official CLIs, not raw APIs
`claude` and `codex` both sign in with a consumer subscription, which bills
the plan instead of per token, and they bring connectors/MCP, tools and file
access for free. Cost: we depend on their output formats, so parsers are
isolated and fixture-tested. Personal use only.

## D-005 Default hotkeys
Tapping the Windows key alone opens the launcher; Win+<key> combos still work.
Ctrl+Space is bound as well. Both are configurable, and the Win override can be
turned off. Default AI provider: Claude.

## D-006 One MCP registry for both AI providers
The app owns a list of MCP servers and writes it into Claude's and Codex's
configs. Add a service once and both providers get it. claude.ai connectors
still come through Claude's login on top of this.

## D-007 Approvals mechanism (resolved by D-018)
Need a way to pause a tool call and ask in the popup. Options:
- Claude: `--permission-prompt-tool` pointed at a tiny MCP tool our app
  serves; or move to the Claude Agent SDK (Node sidecar) with `canUseTool`.
- Codex: its approval / sandbox modes, or its app-server protocol.
Fallback if neither works cleanly: read-only by default, plus an explicit
"allow writes for this conversation" toggle.

## D-008 Built-in Google integrations (no AI) deferred (resolved by D-040)
Commands like `cal` need our own Google OAuth (a Google Cloud project).
Asking Claude covers this for now; revisit in M8.

## D-009 OS-drawn corners and Acrylic, not CSS glass
CSS `backdrop-filter` can't blur the desktop behind a window, so the blur has
to come from the OS (Acrylic via `window-vibrancy`). The OS blur fills the
whole window rectangle, so the corners come from Windows 11 too (an
undecorated window with `shadow: true` gets 8 px rounded corners and a
system shadow). That rules out the 16 px CSS radius from the original plan.
We call `window-vibrancy` directly instead of Tauri's `set_effects` because
the latter drops errors, and the UI needs to know whether glass is on.

## D-010 Two window heights, not fit-to-content on every keystroke
Resizing a blurred window on each keystroke risks flicker and lag. The
window is either the search bar alone or bar + fixed results area + footer,
so it resizes only when the query goes from empty to non-empty and back.

## D-011 Windows key: swallow the real key-up and replay it masked
Injecting a mask key from inside the hook and then letting the real Win-up
through doesn't reliably beat Start, because injected input is processed
after the current event. Swallowing the real Win-up and injecting
mask-down, mask-up, Win-up in one `SendInput` keeps the order exact. If the
injection fails, the real Win-up goes through, so the key can never stick.

## D-012 Windows key: mask on key-down instead (replaces D-011)
On real hardware, masking at key-up still let Start (or search) open now and
then. The masking key now goes in the moment Win goes down, tens of
milliseconds before any release, which is the approach AutoHotkey users rely
on for Windows 11. Win-up is never swallowed or replayed any more, which also
removes the stuck-key risk D-011 had to guard against. Remaining gap: while
an elevated (admin) window has focus, Windows doesn't send keys to our hook
at all, so Start opens normally there.

## D-013 Apps from the AppsFolder, not by scanning .lnk files
The shell's AppsFolder is exactly Start's "All apps" list: desktop programs
and Store apps together, with display names and a launchable id for each.
Scanning Start Menu shortcuts would miss Store apps and need a second code
path (package manager APIs) to cover them. Launching through the same
AppsFolder item also handles Store apps, pinned AUMIDs and `runas` uniformly.

## D-014 Icons over a custom protocol, not data URLs in results
Results are sent on every keystroke; inlining base64 PNGs would bloat each
response and re-decode images constantly. Rows carry a short key instead and
the webview fetches `icon://` URLs, which it caches like any image. Icons are
extracted lazily, so only rows that are actually shown cost anything.

## D-015 Windows key: masked replay of every release (replaces D-012)
D-012 was wrong: masking on key-down made Start open on every tap. The mask
was injected from inside the hook while Win-down was being handled, and
Windows processed it *before* that Win-down, so it didn't count. (D-011's
reasoning had this ordering backwards too.) Swallowing the real Win-up and
injecting mask-down, mask-up, Win-up as one batch is correct whichever way
Windows orders hook-injected input, which is why D-011 mostly worked. It now
applies to every Win release we saw go down, not just detected taps, so a
press misjudged as a combo can't let Start through. Taps alone open the
launcher. Remaining gap: admin windows (see D-012).

## D-016 Everything SDK compiled in, not the DLL (updates D-003)
The SDK is one MIT-licensed C file. Compiling it into the app (`build.rs`,
Windows only) means no `Everything64.dll` to bundle, find or keep in sync,
and a plain `extern "system"` binding for the dozen functions we use. The
source was taken from the `everything-sdk-sys` crate's vendored copy because
voidtools.com wasn't reachable from the build environment; see
`third_party/everything-sdk/README.md`. Considered: the pure-Rust
`everything-ipc` crate (heavier, newer, less proven) and hand-writing the
WM_COPYDATA protocol (error-prone without being able to test locally).

## D-017 Built-in file index; Everything optional (updates D-003, D-016)
Requiring a second background program for file search was poor design for a
personal launcher. The launcher already runs all the time, so it keeps its
own index of the user's folders: scanned on a background thread, saved to
disk so it's searchable the moment the app starts, and kept current by
watching for changes. Fuzzy-scoring 300k names takes ~15 ms, inside the
per-keystroke budget. Everything is still used whenever it's running, since
it covers every drive. Considered: the Windows Search index (already
running, but slow per query and limited to indexed locations) and reading
the NTFS master file table ourselves (what Everything does: needs admin
rights, so effectively another service).

## D-018 Approvals over Claude Code's stdio permission prompt (resolves D-007)
`--permission-prompt-tool stdio` makes Claude Code send each permission
question to us as a `can_use_tool` control request on stdout, and wait for a
`control_response` on stdin. It's the same protocol the Claude Agent SDK's
`canUseTool` uses, so no Node sidecar and no MCP server of our own are
needed. Claude Code's own rules still decide what needs asking, so reads and
harmless commands don't interrupt you. "Auto" mode is just
`--permission-mode auto`. Verified against real sessions: a write waits for
the answer, a deny leaves the file untouched and Claude says so. Considered:
an MCP permission tool served by the launcher (an extra server for the same
result) and the Agent SDK in Node (a second runtime to ship).

## D-019 Screenshots never include the launcher
The opt-in screenshot (M4) is for "what am I looking at", so the launcher
covering part of the screen would only get in the way. Capture the window
that was in front before the launcher opened, by its window handle, which
can't include the launcher even where they overlap. For a full-screen
capture, set the launcher's display affinity to exclude it from capture
(`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`, Windows 10 2004+) for
the moment of the capture only, so your own screenshots still show it.

## D-020 Slow search results arrive late instead of being dropped
Providers used to be skipped for a keystroke when they missed the 40 ms
budget. A big file index (190k entries in testing, more on a real user
folder) missed it on every keystroke, so files never showed up at all. Now
the first response still goes out at 40 ms, a late provider keeps running,
and its results replace the list through a `search-updated` event when
they're ready. The index search also got faster (parallel across cores)
and cancellable (a newer keystroke stops older searches), so late results
are rare and cheap. Considered: just raising the budget (every keystroke
would wait for the slowest provider) and incremental narrowing (re-score
only the previous matches as you type; faster, but more state to keep
right).

## D-021 Desktop context goes in the message, captured as the launcher opens
"Move this file here" needs to know what was on screen. The window in front
is read the moment the launcher is asked to open, before it takes focus
(after that, the launcher is the foreground window). Explorer's folder and
selection need a cross-process COM call, so that part runs in the background
and asking waits for it briefly instead of slowing the launcher down. The
context is put in front of the message rather than in the system prompt,
because it changes each time the launcher opens while the conversation (and
its process) carries on. It's on by default since it's the point of the
feature, shown under each message ("Sees: …") so it's never hidden, and can
be turned off in settings. Considered: an MCP tool Claude calls to fetch the
context (still planned, for a fresh read), but on its own Claude would often
not think to call it.

## D-022 Windows key: hide a lone tap from Windows entirely (updates D-015)
Quick taps still opened Start now and then. With D-015 Windows saw every Win
press and only the release was masked, so anything that let one release
through unmasked opened Start: chiefly Windows skipping our hook when it
answered late (the app busy indexing or searching), since a press the hook
never saw was passed through on release. Now a Win press is hidden until
another key shows it's a shortcut, at which point Win is injected just
before that key. A lone tap is never seen by Windows at all, so there is
nothing for Start to react to; the D-015 masked release remains for presses
Windows did see. The hook thread also runs at time-critical priority. Cost:
Win+L can't be passed on this way (Windows ignores an injected Win for
locking), so the hook locks the PC itself. Considered: turning off Start
(no supported switch; the `NoWinKeys` policy also kills every Win shortcut)
and killing the Start menu process (fragile, and it restarts).

## D-023 The launcher's tools for Claude: MCP over local HTTP
Claude needs to act on the desktop on demand (search with our index, open
and reveal files, a fresh look at Explorer's selection, a screenshot). MCP
is how Claude Code takes extra tools, and serving it from inside the
launcher means the tools use the live index and desktop state directly.
Transport: Streamable HTTP on 127.0.0.1 with a random port and a secret
token per run, written to a config file Claude Code is pointed at with
`--mcp-config`; plain JSON replies, no event stream. Requests carrying an
`Origin` (browsers) or another host name (DNS rebinding) are refused, so a
web page can't drive it. Approvals come from a `--settings` file: reading
tools are pre-approved, the screenshot always asks (an `ask` rule holds
even in auto mode; verified with Claude Code), the rest follow the usual
rules. Considered: a stdio MCP server (a second copy of our exe that would
need its own channel back to the running launcher) and passing the config
as a JSON argument (quoting through an npm `.cmd` shim is fragile).

## D-024 Warm start only while a question is being typed
A Claude Code process that's already running answers about a second sooner
(measured: first words at ~1.3 s instead of ~2.5 s; more with connectors to
load). Keeping one running all the time costs memory, and starting one on
every launcher open costs a process start for each app or file search. So
the UI asks for a spare only once the query looks like a question for
Claude, and an unclaimed spare is closed after 2 minutes. The spare is
started with the options the question would use; if those changed in the
meantime (another Explorer folder, another model), it's replaced rather
than used with stale settings.

## D-025 Re-install the Windows key hook every few seconds
Start kept opening on a real PC even with D-022. The likely cause: since
Windows 7, a low-level hook that once answers too slowly is removed for
good without notice (and every tap then goes straight to Start), and
Windows calls the newest hook first, so another app's hook installed later
sees the key before ours. Re-installing every 5 s while no Windows key is
held covers both: a removed hook comes back within seconds, and ours is
the newest again. It costs a couple of system calls. The settings menu
shows whether the hook is running and when it last saw the key, to tell
these apart from anything else.

## D-026 Settings live in the launcher, saved as you change them
A separate settings window would be one more window to manage for a few
switches. The menu replaces the results area like the chat does, so it
works with the keyboard and the same look. Changes save immediately (no
Save button) and apply live where they can; the two that shape the file
index offer a restart instead of half-applying. The TOML file stays the
source of truth and is still one command away.

## D-027 Features plug in through their own folder and an activation result
M5–M8 add a dozen features that each bring results, actions, settings and
Windows code. Putting each in `features/<name>/` with a `provider`, a
`start` and its own settings struct keeps them from editing the same
files, so they can be built (and reviewed) in parallel. Picking a result
goes to the provider that `owns` its id, which returns an `Activation`
describing what follows (hide, copy, paste, open a link, a new query, ask
the AI, a notice, refresh). The common endings are carried out in one
place (`actions::carry_out`), so pasting into the window you came from and
the UI's follow-up behave the same for every feature. A provider can
`claim` a prefix (`kill `) to be the only one answering it. Risky commands
ask in the footer ("Press Enter again") rather than with a dialog, which
keeps everything on the keyboard. Considered: one big `match` on id
prefixes in `actions.rs` (every feature editing the same function) and
separate IPC commands per feature (more bindings, the same follow-up logic
copied around).

## D-028 Calculator: fend behind a cautious trigger; ECB rates over WinHTTP
The calculator uses fend-core rather than a small expression parser with hand-written unit tables. fend already covers arithmetic, about 1000 units ("3 cups in ml", "1 km^2 in mi^2"), number bases and currencies through a rate callback. It also has an interrupt hook, which lets each evaluation stop after 25 ms.

An answer row always ranks first, so a wrong answer steals Enter from the app or file you meant. For that reason we decide whether to evaluate before fend sees the query. The query needs a digit plus an operation (an operator, a conversion, a function). Queries shaped like dates, versions, phone numbers, hyphenated names or one-letter prefixes are left alone. A leading `=` overrides all of this. Small rewrites (currency symbols, `f`/`c` temperatures, `in` as inches, lower-case file sizes, `x` for times, desk-calculator percentages) make everyday typing work.

Exchange rates come from the European Central Bank's daily file. It's official, needs no key and has kept the same address for many years, but it covers only about 30 currencies. It's downloaded at most once a day with WinHTTP and saved to disk, so conversions work offline and search never downloads. WinHTTP comes with Windows and uses the system's certificates and proxy, which avoids adding a TLS stack to the app for one small file.

Considered:
- meval/evalexpr: no units.
- open.er-api.com: about 160 currencies, but with terms of use and attribution.
- reqwest with rustls: a large dependency for a single request.

Related: emoji rows don't get the ranker's exact-name tier, so the single unprefixed emoji suggestion ("fire") can never sit above Firefox.

## D-029 Clipboard history reads the clipboard itself, writes through arboard, and recognizes its own writes by sequence number
Clipboard history needs three things from a read:
- hold the clipboard only for a moment (apps that copy twice in a row fail while another program has it open);
- check the privacy formats password managers set;
- get the raw formats (`PNG`, `CF_DIBV5`, `CF_DIB`, `CF_HDROP`).

So the listener reads with its own Windows code. It copies the bytes, closes the clipboard, and decodes later on another thread, with pure PNG/DIB/file-list parsers tested on Linux.

Writing images (PNG + DIBV5) and file lists (`CF_HDROP`), and marking a write private, goes through `arboard`. It is the crate behind `tauri-plugin-clipboard-manager` and is already in the dependency tree at the same version. The plugin only offers text, and its image write needs tauri's `image-png` feature and still has no file lists or private flag.

Our own writes (pasting an entry back, a snippet's temporary text, and putting the clipboard back afterwards) are recognized by Windows' clipboard sequence number, not by content. So they never reach history; pasting an entry moves it up directly.

Repeats are found by hash, and images hash their pixels, so the same picture copied from another app doesn't add a second entry. Clip rows keep history order in `rank.rs` (pinned, then newest); frecency and name bonuses would otherwise shuffle the list.

Snippets expand as you type by erasing the keyword and pasting through the clipboard, then putting the clipboard back. Typing the text as Unicode key events is slow for long text, and editors' auto-indent and autocomplete mangle it. The hooks only queue events, so typing is never slowed.

Considered:
- arboard for reading: it holds the clipboard open while decoding an image.
- The plugin for images: no file lists or private flag.
- Skipping our writes by comparing content: a re-encoded image differs.
- Raw input instead of hooks: it can't tell our injected keys apart as reliably.

## D-030 ChatGPT through `codex app-server`, approvals through its sandbox (resolves D-007 for Codex)
`codex exec --json` has no streamed text, forces approval policy "never" and needs a new process per message. So ChatGPT goes through `codex app-server`: JSON-RPC over stdio, one long-running process per conversation like Claude Code, with streamed text, approval requests we answer, `turn/interrupt` and `thread/resume`.

Every thread uses approval policy "on-request", with approvals sent to the user and never to Codex's auto-reviewer. That reviewer decides on its own, uses quota, and would also approve the screenshot.
- "Ask" runs in a read-only sandbox: reading and harmless commands go ahead, and anything that writes or goes online comes to the Approve card. This matches Claude's default.
- "Auto" is Codex's own workspace-write sandbox in the Explorer folder the question came from, with no network; it asks for anything outside. When there's no folder it stays read-only, because the whole home folder is too much to hand over.
- The launcher's screenshot tool is set to "prompt", so it asks in both modes.
- On Windows this needs Codex's no-admin Windows sandbox. The launcher turns it on per process unless the user's Codex config picks one; without a sandbox Codex asks for every command.

Secrets never go on a command line or into Codex's environment, which the commands it runs inherit. The MCP server's token goes in the per-thread config over stdin. An API key goes through `account/login/start` into an in-memory credential store, leaving the ChatGPT sign-in on disk untouched. Codex is never given `forced_login_method`, which signs the user out on a mismatch.

Considered: `codex exec --json` (no streaming, no approvals), Codex's MCP server mode (removed in 0.163), and the auto-reviewer for Auto (decides alone, costs quota, would take over the screenshot prompt).

## D-031 Windows key: close Start when it opens anyway, and fix what makes it open (updates D-022, D-025)
On the owner's Windows 11 PC, a tap opened both the launcher and Start. So the hook saw the tap, but Windows used the key anyway. The only known way that happens is a late answer: Windows stops waiting for a low-level hook after `LowLevelHooksTimeout` (about 300 ms when unset) and passes the key on. Our callback still finishes and reports a tap.

The same cause hid two bugs:
- A press answered late plus a release answered in time left Win stuck down in Windows.
- A release that never reached the hook left the detector "held" for good, which stopped the D-025 re-install.

So:
1. **Make late answers rarer.** No Tauri code or allocation in the hook callback (taps are posted to a guard thread). The hook thread opts out of EcoQoS and locks its stack and code pages in memory. Each Windows key event's answer time is logged, so a timeout can be proven. An optional button sets `LowLevelHooksTimeout` to 1 s.
2. **Fix both bugs.** At a release, `GetAsyncKeyState` says whether Windows got the press; if so it is still a tap, but the release is replayed masked. A held key that has gone quiet, and that Windows says is up, is forgotten.
3. **Safety net, since no hook can promise an answer within 300 ms.** For 1 s after a handled tap, if Start or Search comes to the front, the guard closes it with one Escape and brings the launcher back.
   - Detection: a foreground WinEvent hook, IAppVisibility and a 25 ms poll.
   - Matching: by the shell host's program path under SystemApps, because titles are localized and the window class is shared with other shell UI.
   - Escape: sent only after checking again that Start/Search is in front and no modifier is held. Ctrl+Esc opens Start, so a modifier means hands off.
   - It doesn't act on a Start that follows a Windows key press or Escape, or one that was open before the tap.
   - Cost: a Start you open by mouse within that second gets closed.
   - `hotkeys.close_start` turns it off.

**Considered:**
- Turning Start off. There is no supported switch, and `NoWinKeys` also kills Win shortcuts.
- A Scancode Map that remaps the Win keys to F24. It needs admin rights and a reboot, and leaves no Windows key while the app isn't running.
- Killing or suspending StartMenuExperienceHost (rejected in D-022).
- The undocumented `IImmersiveLauncher::Dismiss`; its interface can change in any build.
- A message hook injected into explorer to drop the Start message. It needs a DLL, and a crash in it takes explorer down.
- Toggling Start with another Win press or `SC_TASKLIST`. If Start had already closed itself, that would open it.

## D-032 Script commands: an in-memory index followed with notify, output handled by mode, batch files quoted by Rust's standard library
Scripts in one folder become commands through a header comment (`@title`, `@mode`, `@argument`, ...), like Raycast's script commands. The folder is read in `start()`, read again on every change (notify), and on a folder setting change. Settings have no change notification, so that is checked once a second. Search only reads the in-memory index, so it stays inside the 40 ms budget whatever is in the folder.

What a script prints goes where its `@mode` says:
- silent: hide, then a toast with the last line.
- copy / paste: through `Activation::Copy` / `Paste`, so pasting behaves like every other feature.
- output: a footer notice when it fits, else a text file opened in the editor. The footer shows only about one line next to the key hints, and a cut-off result would be lost without anyone noticing.

Batch files are started directly rather than as `cmd /C <file> <text>`. Rust's standard library then runs them through cmd.exe and escapes the argument for cmd, so `&` or `|` typed after a title can't run as a command. A time-out stops the whole process tree (`taskkill /T /F`), since scripts often start other programs. After the script exits, its output is read for at most 500 ms more, so a program it left running can't hold the result.

Considered:
- Running every file in the folder without a header (helper files would show up as commands).
- A Job object per script to stop it (more Windows code for the same effect as taskkill here).
- Showing long output as result rows (a new UI mode for one feature).

## D-033 Updates: Tauri's signed updater, set up per build
The Tauri updater checks a signature on every download, so a hijacked
release or a swapped file can't install itself. The catch is the key pair:
the private key has to live in a GitHub secret that only the owner can
add. So the public key is fixed at build time from a repository variable,
and a build without one (dev builds, every `build.yml` build, a fork)
simply says "Updates aren't set up for this build" and never checks. The
plugin's config section is filled in at startup so a missing key can't
stop the app. Downloads happen in the background, but installing always
waits for the owner (it restarts the app and the AI sessions). Considered:
checking the GitHub Releases API ourselves and running the installer
(no signature check, or a hand-rolled one) and a Microsoft Store/winget
listing (heavy for a personal app).

## D-034 System tools call Windows directly; the color picker borrows the cursor instead of covering the screen
Lock, sleep, restart, the Recycle Bin, dark mode, ending processes and moving windows call Win32 directly (`LockWorkStation`, `SetSuspendState`, `ExitWindowsEx`, `SHEmptyRecycleBinW`, the Personalize registry values plus a `WM_SETTINGCHANGE` broadcast, `TerminateProcess`, `SetWindowPos`). The alternative was running `shutdown.exe`, `rundll32` or PowerShell. Direct calls flash no console and depend on nothing in PATH, and failures come back as codes the footer can explain ("it runs as administrator"). Sleep on Modern Standby PCs, which have no classic sleep for `SetSuspendState` to ask for, turns the display off as the power button does.

The color picker replaces the system cursors with a crosshair and catches the next click with low-level mouse and keyboard hooks on its own thread, then reads one pixel in physical pixels. Considered: a full-screen overlay window over a screenshot (it could show a magnifier, but needs a window per monitor at that monitor's DPI and captures the whole screen for one pixel). Replaced cursors outlive a crash, so a marker file in the data folder brings them back at the next start.

Now Playing comes from WinRT's media sessions on a watcher thread, not from inside `query`, because asking another app's session can take longer than the 40 ms search budget.

## D-035 Chat history is recorded in Rust and reopened as an idle conversation
Chats are recorded from the backend's own event stream rather than by the UI: the backend sees every event even while the launcher is hidden or the webview reloads, and saving after each turn bounds what a crash can lose. History has its own SQLite file instead of sharing `data.db` with frecency, so it can be cleared on its own, has its own migrations, and its heavier writes never contend with search. All database work runs on one writer thread, in order, so streaming never waits for the disk and a delete can't be undone by an older save still queued.

Reopening doesn't replay the transcript to the AI. It hands the stored session or thread id to the chat as the current, idle conversation (`Chat::adopt`), and the existing restart-after-idle path resumes it (`--resume` in its folder, `thread/resume`). Claude Code deletes old sessions, so when its session file is gone the chat starts over and says so. Considered: saving from the UI over IPC (misses events while hidden), storing in `data.db`, replaying the transcript into a new session (costly, and still not the same conversation).

## D-036 Synced settings are two layered files, with live reload that recognizes our own writes
Settings that should follow you between PCs and settings that belong to one PC live in different files. `settings.toml` can sit in a synced folder (OneDrive); `settings.local.toml` stays in `%LOCALAPPDATA%` and is layered on top with the same merge as the defaults. A fixed list of keys (`MACHINE_KEYS`: paths on this disk, the index, start at login, Windows key behaviour) is always saved locally, and so is any key added to the local file by hand. Saving merges into what each file holds, so keys from a newer version survive. The chosen folder is remembered in a local pointer file, never inside the synced folder, so each PC can choose differently. Changes from the sync are applied while running. The app compares each file's text with what it last read or wrote rather than trusting timestamps or ignore windows, because sync clients and atomic renames make those unreliable; a 30 s check backs up the watcher for folders that come and go. Considered: one synced file with per-PC values marked inside it (a sync would overwrite them), a per-machine section keyed by PC name (grows forever, hard to edit by hand), and Tauri's store plugin (no layering, no hand edits).

## D-037 MCP registry secrets: Credential Manager, then each AI's own channel
The registry lives in `settings.toml`, which can sync between PCs, so secret values (tokens in env vars and headers) can't be stored in it. They go in Windows Credential Manager on each PC, named after their server and entry; the settings only name them, and serializing blanks them in case one slips through. Each AI then gets them the safest way it accepts. Codex: in the per-thread config over stdin, like the launcher's own token (D-030), never on a command line or in an environment the model's commands inherit. Claude Code reads MCP servers only from a file or a command-line argument, so the launcher writes them into the `--mcp-config` file it already uses (D-023): in the local data folder (never synced, readable only by the user), rewritten only when it changes, deleted when empty. Considered: `${VAR}` placeholders with the values in Claude Code's environment (every command Claude runs would inherit them), the JSON as a `--mcp-config` argument (visible in process listings; fragile quoting through npm shims), a temporary file per process (no reliable moment to delete it), and the `keyring` crate (a dependency for three small calls).

## D-038 Placement follows tao's DPI handling; idle memory is given back after two minutes hidden
The launcher's size is always set in logical pixels, and its position is set before and after the size. tao converts a logical size with the scale it currently knows for the window, which is right whether Windows sends WM_DPICHANGED during the move or only when the window shows; a physical size would be rescaled a second time. A scale change while visible refits the window shortly after. Placement is a pure function that moves the window up rather than letting results run off a short screen.

While hidden, the window keeps its WebView2 (that keeps hotkey-to-visible fast). After two minutes hidden it asks WebView2 for its Low memory target and trims the process working set, then sets Normal before showing: a few soft page faults on the first open after idle, for a smaller idle footprint. Considered: destroying and recreating the window (slow to show), TrySuspend (needs the controller hidden and pauses events), physical sizes (double-scaled when the DPI change comes late).

## D-039 AI inputs travel as attachments on the question, not as text pasted into it
Selected text, files and pictures become typed attachments that the UI shows as chips and the backend turns into each AI's own message shape. Text and paths are tagged blocks in front of the question (like the desktop context). Pictures are saved once as PNG files in the data folder: Claude Code gets the bytes inline as image content blocks, and Codex gets the path as a local image, since it reads the file itself. Keeping them apart from the typed text lets you see and remove what's attached before asking, keeps the chat bubble and history to what you typed, and keeps pictures out of IPC (the UI only holds an id and a preview). The selection is read by pressing Ctrl+C and watching the clipboard's sequence number, with the old clipboard put back, because UI Automation selection reading doesn't work in many apps. Screen regions use Windows' own snipping overlay (`ms-screenclip:`) and the clipboard. Considered: pasting the selection into the search box (it searches, and long text is unreadable there), pictures as base64 over IPC, and our own region-capture window (DPI and multi-monitor cases the snipping tool already handles).

## D-040 Google integrations through your own OAuth client: read-only scopes, loopback PKCE, WinHTTP (resolves D-008)
`cal`, `mail` and Drive search need Google's APIs, which need an OAuth client. A shared, verified client isn't practical for a personal app (Gmail scopes are restricted; verification needs a security assessment), so each install uses its owner's own "Desktop app" client, made once in Cloud Console. Sign-in follows RFC 8252: Authorization Code with PKCE (S256) and a one-shot listener on 127.0.0.1 at a random port, with a `state` check and a 5-minute limit. Scopes are the narrowest that do the job: `calendar.events.readonly`, `gmail.metadata` (headers, never bodies), `drive.metadata.readonly`, plus `openid email`. The cost: Gmail refuses search under `gmail.metadata`, so `mail <text>` filters cached recent threads locally and offers Gmail's web search. The client secret and refresh token stay in Credential Manager; access tokens only in memory. Requests reuse WinHTTP (now `net.rs`), so there is still no TLS stack in the app (D-028). Calendar and inbox are cached in the background, so search never waits; Drive is asked per query after a pause and delivered as late results (D-020). Considered: `gmail.readonly` (search, but exposes message bodies), yup-oauth2 / google-apis crates with hyper-rustls (a large dependency tree for a few GETs), the device-code flow (Google doesn't allow Gmail or Drive scopes with it), a shared Cloud project (verification, quotas shared with strangers).

## D-041 Borrowing the clipboard puts back every format; injected keys only go where they were meant
Snippet expansion and the selection shortcut borrow the clipboard. They now save every memory-block format on it (`clipboard::system::save`) and put all of them back, kept out of clipboard histories, so an Excel range or Word formatting survives; before, only one format (text, a picture or a file list) came back. Snippet expansion erases the keyword before saving the clipboard (saving can wait on the app that copied) and presses keys only while the keyword's window is still in front. Pasting from the launcher no longer presses Ctrl+V when the window you came from can't be brought back: the text is left on the clipboard with a toast. The selection shortcut copies with Ctrl+Insert in terminals, where Ctrl+C with nothing selected would stop the running program. "Empty Recycle Bin" always asks while asking is on, because the cached "already empty" can be stale. Codex's Auto mode stays read-only in a drive root, your home folder or above it, and Windows' own folders. The idle working-set trim is skipped while a keyboard hook runs, so the hook's data isn't paged out (D-031).

## D-042 Compare runs two live conversations in one Chat, in a wider window, saved as two linked chats
Asking Claude and ChatGPT side by side reuses everything a single chat has (spares, approvals, stop, idle shutdown, resume, history) by letting `Chat` hold up to two live conversations rather than adding a second chat object. Events already carry a conversation id, so the UI routes them by column, and approval ids are unique across the two CLIs, so `respond` just asks each side. The single-chat API keeps its meaning by acting on the first (only) conversation. The window grows to 1040 px while comparing, once on entering and once on leaving like the two-height switch (D-010), so each column gets about 480 px; two 320 px columns would wrap most code and tables, and tabs would hide the comparison that is the point. History saves each side as its own chat, linked while both are live. The `vs` prefix needs three or more words so it can't take over "vs code". Considered: a second `Chat` instance (duplicate spare and idle logic, two event streams to merge), tabs with Ctrl+1/Ctrl+2, and one combined history entry.
