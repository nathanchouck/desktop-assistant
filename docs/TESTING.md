# On-device test list

Everything below has passed the Linux tests and the Windows lint, but none of
it has run on Windows yet. Work through it on your PC and tick what works.
When something doesn't, see "If something breaks" at the end.

Shortcuts used below: **Enter** runs the highlighted row, **Ctrl+K** lists
every action for a row, **Esc** backs out one step.

## 0. Before you start

- [ ] Install the latest build: GitHub → Actions → the newest green **build**
      run for `claude/hopeful-goldberg-h3f01a` → Artifacts →
      `desktop-assistant-windows` → run the `.exe` installer.
- [ ] Start it. A tray icon appears. Logs are in
      `%LOCALAPPDATA%\desktop-assistant\logs` (tray → Open Log Folder).
- [ ] For the Windows key tests, set a user environment variable
      `RUST_LOG=desktop_assistant_lib=debug` and restart the app, so the log
      has timings.
- [ ] Have Claude Code installed and signed in. For section 4, also install
      Codex (`npm i -g @openai/codex`).

## 1. Opening the launcher and the Windows key

- [ ] Ctrl+Space opens and closes the launcher, centered on the monitor with
      the mouse, about a fifth of the way down.
- [ ] Tap the Windows key 20 times, at different speeds, including after the
      PC has been idle for a few minutes. Each tap toggles the launcher;
      **Start never stays open**. If Start flashes, it closes within about
      0.1 s and the launcher is back in front.
- [ ] Settings → "Tap the Windows key" says "Working: saw it … ago". If Start
      was closed or a key was answered late, it counts them.
- [ ] Win+E, Win+V, Win+Shift+S, Win+L, Win+D and Win+Tab all still work, also
      right after a tap. Tap, then Win+S within a second: Search stays open.
- [ ] Clicking the Start button with the mouse, or Ctrl+Esc, opens Start
      normally and it stays open.
- [ ] Stuck-key check: after a busy moment (a big search), tap Win a few
      times, then type `e` and `d` in Notepad. Letters type; Explorer doesn't
      open and the desktop doesn't show.
- [ ] Optional: Settings → "Give the Windows key more time" → Allow 1 s, sign
      out and back in, and see if Start leaks less. Undo removes it.
- [ ] Clicking another window hides the launcher; Esc clears the text, then
      hides.

## 2. Apps and files

- [ ] Your apps show up (no uninstallers), with sharp icons in light and dark
      mode. "vsc" finds Visual Studio Code.
- [ ] Enter launches and brings the app to the front; Ctrl+Shift+Enter shows
      the UAC prompt for a desktop app.
- [ ] Apps you open often move up within a day.
- [ ] Without Everything: files in your user folder show up as you type (on
      first launch the footer shows the scan's progress). A file you just
      saved or renamed shows up within a few seconds.
- [ ] With Everything running: files anywhere on your drives show up.
- [ ] Nothing from AppData, `node_modules`, `.git` or Temp shows up.
- [ ] On a file: Ctrl+Enter shows it in Explorer, Ctrl+Shift+C copies its path.
- [ ] Ctrl+K on a file lists Open, Show in Folder, Copy Path, Open With…,
      Ask AI About This, Delete to Recycle Bin. Typing filters the list.
- [ ] Open With… shows Windows' "How do you want to open this file?" dialog.
- [ ] Ctrl+D on a file asks in the footer; Enter again moves it to the Recycle
      Bin and the row disappears. Ctrl+Z in Explorer brings it back.

## 3. Claude

- [ ] Type a question and press Tab: the answer streams in; follow-ups
      continue the conversation; Esc stops a running answer, then closes.
- [ ] Math in an answer renders: ask "show the quadratic formula in LaTeX".
      Prices like "$5 and $10" stay plain text.
- [ ] With 2 files selected in Explorer, ask "what did I select?": the chat
      shows "Sees: <folder> · 2 selected" and Claude names them.
      "Move these into a new folder called Test" asks Approve/Deny
      (Ctrl+Y / Ctrl+N) and works.
- [ ] "Take a screenshot and tell me what you see" asks first, then describes
      the window you came from. The launcher isn't in the picture.
- [ ] "What's on my calendar today?" works through your claude.ai connectors.
- [ ] Sign out of Claude Code (`claude auth logout`): the last row becomes
      "Sign in to Claude Code to ask: …" and Enter opens a sign-in window.
- [ ] Typing a question slowly starts Claude early (claude.exe appears in Task
      Manager before Enter); the answer starts sooner.

## 4. ChatGPT (Codex)

- [ ] Before Codex is installed, `g hello` shows "Install Codex to ask
      ChatGPT"; installed but signed out shows "Sign in to ChatGPT"
      (opens `codex login`). Settings → ChatGPT shows the same status.
- [ ] `g what is the capital of France`: answers, footer says
      "ChatGPT · <model>". No console window flashes.
- [ ] Settings → ChatGPT → Default AI = ChatGPT: Tab asks ChatGPT,
      Shift+Tab asks Claude, `c ` still forces Claude. Switch back after.
- [ ] From an Explorer folder: `g create notes.txt here containing hello`
      shows Approve/Deny with the path and content. Reading files doesn't ask.
      (If even reads ask every time, Codex's Windows sandbox isn't working:
      note it.)
- [ ] Approvals = Auto: changes inside the Explorer folder happen without
      asking; a file on the Desktop asks.
- [ ] `g take a screenshot…` always asks first.
- [ ] Quit from the tray with a ChatGPT chat open: no codex.exe left behind.

## 5. More AI

**History and last answer**
- [ ] After a few chats, `history` lists them newest first ("Claude · 3 min
      ago", "N messages"). `history <word>` finds a chat by its content.
- [ ] Enter reopens a chat; a follow-up continues it (Claude remembers).
      Ctrl+Shift+P pins, Ctrl+D deletes. "Clear Chat History" asks first and
      keeps pinned chats.
- [ ] In a finished chat: Ctrl+Shift+C copies the last answer; Ctrl+Enter
      pastes it into the window you came from.

**Inputs**
- [ ] Select text in Notepad, press Ctrl+Shift+Space: the launcher opens with
      the text as a chip; your clipboard is unchanged. Ask "fix the grammar".
- [ ] Same shortcut with 2 files selected in Explorer: two file chips.
- [ ] On a file row, Ctrl+Shift+A: a file chip; ask "summarize this".
- [ ] "Ask About Screen Region": the snipping overlay opens; after a snip the
      launcher returns with a picture chip. Ask Claude, then Shift+Tab for
      ChatGPT: both describe the image.
- [ ] "Dictate" (or the mic button): Windows voice typing types into the box.
- [ ] Saved commands: copy a paragraph, type `summ`, Enter: Claude
      summarizes it. `fix teh cat` fixes the typed text. Settings → AI
      commands lets you edit the list and the selection shortcut.

## 6. Calculator, quicklinks, emoji

- [ ] `2^10` → 1024 as the top row; Enter copies it; Ctrl+Enter pastes it.
- [ ] `5 mi to km`, `72 f to c`, `3 cups in ml`, `200 + 10%`, `255 to hex`,
      `sqrt(2)` all give answers.
- [ ] No answer row for `chrome`, `notes`, `tax 2024`, `2024-01-01`, `7-zip`.
- [ ] `100 usd to eur` works (rates download in the background; offline it
      uses the saved copy).
- [ ] `yt lofi` opens YouTube search; `gg rust` searches Google (`g ` is for
      ChatGPT). Settings → Quicklinks: add your own.
- [ ] `:fire` → 🔥; Enter pastes it into the window you came from;
      Ctrl+Shift+C copies. `:` alone shows recent emoji first. `:arrow`,
      `:degree`, `:check` find symbols. Skin tone in Settings → Emoji.

## 7. Clipboard and snippets

- [ ] Copy some text, type `clip`: it's on top ("notepad · just now").
      Copying it again moves it up instead of adding a duplicate.
- [ ] Enter pastes an entry into the window you came from. Ctrl+Shift+P pins,
      Ctrl+D deletes.
- [ ] Copy an image (Win+Shift+S): `clip` shows it with a thumbnail and pastes
      it back. Copy 2 files in Explorer: pasting puts the files back.
- [ ] A password copied from your password manager never shows up.
- [ ] Settings → Snippets: add `;sig`. Type `sig` in the launcher: Enter pastes
      it. With "Expand as you type" on, typing `;sig` in Notepad replaces it
      with the snippet, and your clipboard is restored after.
- [ ] Typing feels normal with expansion on (no lag).

## 8. System, windows, processes, colors, media

- [ ] `lock` locks. `sleep` sleeps. `restart`, `shut down`, `sign out` and
      `empty trash` ask in the footer first (save your work before trying).
- [ ] `dark mode` flips Windows' theme; again flips it back.
- [ ] `mute` mutes and unmutes. `wifi`, `bluetooth`, `display`, `sound`,
      `windows update` open those Settings pages.
- [ ] From Notepad: `left half`, `right half`, `left third`, `center`,
      `maximize`, `almost maximize`. The window fills that space exactly, with
      no gaps at the edges. Two windows snapped left and right meet exactly.
- [ ] `restore window` puts it back. With 2 monitors, `next monitor` moves it
      over and keeps its layout.
- [ ] `kill notepad` with 2 Notepads open: one row for the program, one per
      window; Enter asks, Enter again ends it. `kill` alone never lists
      system processes or the launcher itself.
- [ ] `pick color`: the cursor becomes a crosshair; click something: its color
      is copied and the launcher shows recent colors. Esc cancels.
- [ ] `#ff8800` shows the color in other formats.
- [ ] With music playing: `pause`, `next track`, `now playing` work.

## 9. Script commands

- [ ] `%APPDATA%\desktop-assistant\scripts` has README.txt and hello.ps1.
      `say hello` runs it; `say hello José` passes the name.
- [ ] Drop a new script (see README.txt) into the folder: it shows up in
      search within a second, no restart.
- [ ] Copy/paste-mode scripts copy or paste their output; a failing script
      shows its error in the footer; no console window flashes.

## 10. Google (optional; needs a one-time setup)

- [ ] Settings → Google: follow the 5 steps to make your own "Desktop app"
      client in Google Cloud Console, paste the client ID and secret, press
      Connect, approve in the browser. Settings shows "Connected as …".
- [ ] `cal`: today's and tomorrow's events; Enter joins the meeting link.
- [ ] `mail`: unread count and newest unread threads; Enter opens the thread
      (check it opens your right Google account).
- [ ] Typing a Drive file's name (3+ letters) adds "Google Drive" rows under
      your local files after a moment.
- [ ] Disconnect removes it; Credential Manager no longer lists the Google
      entries.
- [ ] Note: in Google's Testing mode you'll have to reconnect every 7 days;
      the launcher tells you when.

## 11. Settings

- [ ] Ctrl+, or the "Settings" command opens the menu; changes save as you
      make them; Esc closes.
- [ ] Shortcut recorder: click, press a new combo; it works at once.
- [ ] Appearance: each accent color, glass strength and compact density apply
      instantly, in light and dark mode. A custom hex like `#ffd400` stays
      readable.
- [ ] Advanced → move settings into OneDrive. Edit
      `OneDrive\desktop-assistant\settings.toml` in Notepad (e.g.
      `tint = "green"`): the launcher follows within a second. A typo shows an
      error toast and keeps the old settings.
- [ ] On a second PC using the same OneDrive folder: the look and shortcuts
      follow, but index folders, start at login and the Windows key setting
      stay per-PC.
- [ ] Integrations → API keys: saving a key shows "Saved · ends in …" and
      Credential Manager lists `desktop-assistant:anthropic-api-key`. Billing
      = API key uses it; My plan ignores it.
- [ ] Integrations → MCP servers: add one (e.g. the filesystem server with
      `cmd`, `/c`, `npx`, `-y`, `@modelcontextprotocol/server-filesystem`,
      your Documents path). New Claude and ChatGPT chats can use its tools.

## 12. Errors, monitors, memory

- [ ] Put a typo in settings.toml and restart: opening the launcher within 2
      minutes shows a red toast with "Open log".
- [ ] Monitors at different scales (100% and 150%): the launcher is the same
      visual size and centered on each. Changing the display scale doesn't
      leave it the wrong size.
- [ ] Idle memory: in Task Manager note desktop-assistant.exe and its
      msedgewebview2.exe children, leave the launcher hidden for 3 minutes:
      memory drops. Opening it still feels instant. Idle CPU stays at 0%.
- [ ] With debug logging on, the log shows "ready N ms after start" and a
      timing line per search.

## 13. Updates (one-time setup, then optional test)

Installed copies can only update once you set up a signing key:
1. On your PC: `npx tauri signer generate -w %USERPROFILE%\.tauri\desktop-assistant.key`
   (pick a password or leave it empty).
2. GitHub repo → Settings → Secrets and variables → Actions:
   - secret `TAURI_SIGNING_PRIVATE_KEY` = the contents of the `.key` file
   - secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` = its password (if any)
   - variable `TAURI_UPDATER_PUBKEY` = the contents of the `.key.pub` file
3. To release: set the same version in `src-tauri/tauri.conf.json`,
   `src-tauri/Cargo.toml` and `package.json`, commit, then push a tag
   `v<version>` (e.g. `v0.2.0`). The **release** workflow publishes it.

- [ ] Install the release build. Settings → Updates says "Up to date".
- [ ] Publish a newer version: within a minute of starting (or "Check for
      Updates"), a toast says it's ready; "Install Update and Restart"
      installs it.

## If something breaks

Send me:
- what you did and what happened instead;
- the matching lines from the newest log file in
  `%LOCALAPPDATA%\desktop-assistant\logs` (with debug logging on, if you can);
- for the Windows key: any line mentioning "Start opened", "answered", or
  "late".
