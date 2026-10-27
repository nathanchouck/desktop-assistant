# Desktop Assistant

A personal Raycast-style launcher for Windows. Tap the Windows key (or press
Ctrl+Space) to get a floating glass search bar that finds apps and files
instantly. Anything that doesn't match goes to Claude, or to ChatGPT, billed
to your existing subscriptions, with access to your connectors (Gmail, Google
Calendar, Drive, and so on).

Personal use only: it signs in with my own Claude and ChatGPT accounts and is
not meant to be distributed.

## Status

Milestones 0–3 done: a glass launcher that opens with Ctrl+Space or a tap of
the Windows key, finds and launches your installed apps, and searches your
files with its own index (or every drive through Everything, if it's
running), with icons, fuzzy matching and ranking by what you use. It lives
in the tray.

M4 done: press Tab (or type `c ` before a question) to ask Claude. The
answer streams into the launcher, follow-ups continue the conversation, and
Claude can act on your files, asking Approve / Deny before it changes
anything. It knows which window you came from, and for File Explorer the
folder and the selected files, so "move these into a folder called Taxes"
just works. It can also search with the launcher's index, open or reveal
files, and (each time you approve) look at a screenshot of your window.
Needs [Claude Code](https://code.claude.com) installed and signed in; the
launcher offers to fix either if not.

Settings: type "settings" (or press Ctrl+,) in the launcher. Changes save as
you make them.

M5 done: ChatGPT through OpenAI's Codex CLI, billed to your ChatGPT plan. Type `g ` before a question to ask ChatGPT (`c ` for Claude), pick the default AI in settings, and press Shift+Tab to ask the other one. ChatGPT gets the same screen context and launcher tools, and asks Approve / Deny before changing anything (or works on its own inside the folder you came from, in Auto). Needs the Codex CLI installed and signed in with ChatGPT; the launcher offers to fix either if not.

M6–M8 done (all built; checks on a real PC are in
[docs/TESTING.md](docs/TESTING.md)):
- **AI:** conversation history (`history`), copy or paste the last answer,
  LaTeX math, ask about selected text (Ctrl+Shift+Space), a file, or a
  screen region, dictation, saved AI commands, and Claude vs ChatGPT side
  by side (Ctrl+Tab).
- **Search:** calculator with units and currencies, quicklinks (`yt lofi`),
  emoji (`:fire`), clipboard history (`clip`), snippets, script commands,
  and Google Calendar, Gmail and Drive (`cal`, `mail`) with your own Google
  client.
- **Windows:** system commands (lock, sleep, restart, dark mode, empty the
  Recycle Bin), window layouts, `kill <app>`, a color picker and media keys.
- **Settings:** a synced settings folder (OneDrive) with per-PC values,
  themes, API keys in Credential Manager, an MCP server list shared by both
  AIs, auto-update from GitHub Releases (after a one-time key setup).
- **Polish:** Ctrl+K action menu, Open With and Recycle Bin on files, error
  toasts, multi-monitor/DPI placement, and memory given back when idle.

## Getting the app

Every push builds a Windows installer on GitHub Actions: open the latest
**build** run under the Actions tab and download `desktop-assistant-windows`.

## Developing

Needs Node 22+ and Rust (stable). On Windows, also the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) (WebView2 is
already on Windows 11).

```sh
npm install
npm run tauri dev
```

See [docs/CONVENTIONS.md](docs/CONVENTIONS.md) for the other commands.

## Docs

| Doc | What it covers |
|---|---|
| [docs/ROADMAP.md](docs/ROADMAP.md) | Full todo list, by milestone, including later features |
| [docs/TESTING.md](docs/TESTING.md) | What to check on a real Windows PC, feature by feature |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | How the app is built: processes, modules, data flow, budgets |
| [docs/CONVENTIONS.md](docs/CONVENTIONS.md) | Standard procedures: code style, adding a provider, testing, releasing |
| [docs/DECISIONS.md](docs/DECISIONS.md) | Short records of key decisions and why |

## Requirements (target machine)

- Windows 11 (Windows 10 works, but without the Acrylic glass effect)
- Optional: [Everything](https://www.voidtools.com/) 1.4+ running, to search
  every drive (otherwise the built-in index covers your user folder)
- [Claude Code](https://code.claude.com) installed, signed in with `claude login`
- Optional: [Codex CLI](https://github.com/openai/codex) signed in with ChatGPT
