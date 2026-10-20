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
the Windows key, finds and launches your installed apps and searches every
file on your drives through Everything (with icons, fuzzy matching, and
ranking by what you use), and lives in the tray. Anything else shows an
"Ask Claude" row. Next up is M4 (asking Claude). See the roadmap.

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
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | How the app is built: processes, modules, data flow, budgets |
| [docs/CONVENTIONS.md](docs/CONVENTIONS.md) | Standard procedures: code style, adding a provider, testing, releasing |
| [docs/DECISIONS.md](docs/DECISIONS.md) | Short records of key decisions and why |

## Requirements (target machine)

- Windows 11 (Windows 10 works, but without the Acrylic glass effect)
- [Everything](https://www.voidtools.com/) 1.4+ running (provides file search)
- [Claude Code](https://code.claude.com) installed, signed in with `claude login`
- Optional: [Codex CLI](https://github.com/openai/codex) signed in with ChatGPT
