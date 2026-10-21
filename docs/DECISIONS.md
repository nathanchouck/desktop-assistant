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

## D-008 Built-in Google integrations (no AI) deferred
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
