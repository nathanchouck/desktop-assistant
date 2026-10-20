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

## D-007 Approvals mechanism (OPEN, spike in M4)
Need a way to pause a tool call and ask in the popup. Options:
- Claude: `--permission-prompt-tool` pointed at a tiny MCP tool our app
  serves; or move to the Claude Agent SDK (Node sidecar) with `canUseTool`.
- Codex: its approval / sandbox modes, or its app-server protocol.
Fallback if neither works cleanly: read-only by default, plus an explicit
"allow writes for this conversation" toggle.

## D-008 Built-in Google integrations (no AI) deferred
Commands like `cal` need our own Google OAuth (a Google Cloud project).
Asking Claude covers this for now; revisit in M8.
