# Yunagi User Guide

Yunagi turns your screen into a single sheet of paper for today's work. It keeps only three things, organized by day: what to do today, what is blocking you, and a short reflection at the end of the day.

## 1. Install

### macOS and Linux

```sh
curl -fsSL https://<official-site>/install.sh | sh
```

On macOS you can also use Homebrew:

```sh
brew install <tap>/yunagi
```

If you installed with Homebrew, macOS may block Yunagi on first launch. In that case, remove the download flag (the quarantine attribute) and launch it again (to be confirmed: this step becomes unnecessary if Homebrew removes the flag automatically):

```sh
xattr -dr com.apple.quarantine /Applications/Yunagi.app
```

### Windows (PowerShell)

```powershell
irm https://<official-site>/install.ps1 | iex
```

If you prefer to read the script first, open the URL, review it, and run the saved file. The install script automatically verifies each download against its published SHA-256 hash. If you download a package manually, verify it with `SHA256SUMS` and its minisign signature `SHA256SUMS.sig` from the official site; the steps are on the site's install page.

On Linux, the tar.gz and .deb packages do not update themselves from within the app. To update, install the new version the same way (the AppImage updates automatically).

## 2. First launch

Open Yunagi from your applications. After that, press **Mod+Shift+D** (⌘ on macOS, Ctrl on Windows and Linux) to bring it up from anywhere.

You can change this key in the Operation section of Settings, under Keyboard shortcuts (a letter or digit key combined with Mod or Alt). If another app is already using the key and it can't be registered with the OS, the reason appears under that row in Settings. Choose a different key.

On Linux with Wayland, this shortcut may not work (Settings says so too). If so, assign the command `yunagi show` to a key of your choice in your desktop environment's keyboard settings.

- GNOME: add it in Settings → Keyboard → Keyboard Shortcuts → Custom Shortcuts.
- KDE Plasma: add it in System Settings → Keyboard → Shortcuts → Add Custom Shortcut.
- Hyprland, Sway and similar: add a key binding (`bind`) that runs `yunagi show` to the configuration file.

  ```
  # Hyprland (~/.config/hypr/hyprland.conf)
  bind = SUPER SHIFT, D, exec, ~/.local/bin/yunagi show
  # Sway (~/.config/sway/config)
  bindsym $mod+Shift+d exec ~/.local/bin/yunagi show
  ```

`yunagi show` brings the Yunagi window to the front (it restores it if minimized, shows it if hidden, and moves the focus to it). If Yunagi is not running, it starts Yunagi first. It can take a few seconds for the window to appear after starting.

- If the key binding says the command can't be found, that key setting does not inherit your shell's `PATH`. Write the full location, like `~/.local/bin/yunagi show` in the examples above. Where `~` is not available, use the actual path of your home folder.
- To open Yunagi in Note mode, assign `yunagi show --note` to another key (for example, to start taking notes right away in a meeting).
- You can check the result by running `yunagi show` in a terminal. If Yunagi can't be found, set the environment variable `YUNAGI_APP` to the location of Yunagi.

## 3. A day with Yunagi

1. **Morning**: Yunagi shows the TODOs left over from the last day you used it and blockers that are still open. Pick what you want to carry over. TODOs carried over three times or more (you can change the number in Settings) come with "Split" and "Drop" options, a good moment to rethink them.
2. **During the day**: In Board mode, work through your TODOs from the top. The order is the priority. Write down anything that's blocking you under "Blockers".
3. **In meetings**: Press **Mod+J** to switch to Note mode and write freely. **Mod+Shift+H** inserts a timestamped heading such as "10:00 Standup".
4. **After meetings**: On a line that became a task, press **Mod+Shift+T** to send it to TODOs. On a line that describes a problem, press **Mod+Shift+B** to send it to Blockers. Press **Mod+J** again to go back to Board mode.
5. **End of day**: Write a short reflection: what went well, what got stuck, and your first move for tomorrow (**Mod+Shift+R** inserts the headings).

Everything is saved automatically. There is no save button.

The meter at the top of the screen shows your day as a sky and a sea. The sun marks the current time (it moves from 9:00 to 19:00 by default; you can change this in Settings → General → Nagi meter hours), the waves show the work that's left, and the path of light on the water shows the work you've finished. When nothing is left, the sea goes flat and the meter reads "NAGI".

The sidebar on the left holds Today, Archive, Recent days, and Settings. Press **Mod+\\** to open or close it. It closes automatically in Note mode.

To look back at past days, press **Mod+Shift+L** to open Archive. It shows each day of the week (Mon–Fri) along with the "went well" and "got stuck" lines from your recent reflections, and the reflection and note headings of the selected day. Use ← / → to pick a day, **Mod+[ / Mod+]** to move by week, and Enter to open that day. Press Esc (or **Mod+Shift+L**) to go back.

## 4. Key shortcuts

| Action | Keys |
| --- | --- |
| Bring up Yunagi | Mod+Shift+D |
| Note mode ⇄ Board mode | Mod+J |
| Send line to TODOs / Blockers | Mod+Shift+T / Mod+Shift+B |
| Toggle done / resolved | Mod+Enter |
| Copy the selected TODO as a request | Mod+Shift+A |
| Reorder items | Alt+↑ / Alt+↓ |
| Previous / next day / today | Mod+[ / Mod+] / Mod+T |
| Undo / redo | Mod+Z / Mod+Shift+Z (Ctrl+Y also works on Windows and Linux) |
| Search and commands | Mod+K |
| Open / close the sidebar | Mod+\\ |
| Archive | Mod+Shift+L |
| Settings | Mod+, |

The shortcuts listed here can be viewed and changed in Settings → Input → Keyboard shortcuts. A key can't be shared by two operations that work in the same place, and the keys for undo, redo, copy and paste can't be changed.

## 5. Vim mode

Enable it in Settings → Input → Vim mode. Notes and reflections support Normal, Insert, and Visual modes. In notes, `<Space>t` sends a line to TODOs and `<Space>b` to Blockers (in Visual mode, the selected lines), and `<Space>h` inserts a timed heading so you can type its title right away. In reflections the `<Space>` keys do nothing. The current mode is shown in small letters at the bottom right of the editor. In Board mode, use j / k to move, o to add, i to edit, x to toggle done/resolved, dd to delete, and u to undo. When you leave Insert mode, your Japanese (or other) input method switches back to direct input automatically.

On Windows and Linux, enabling Vim mode changes previous/next day (and previous/next week in Archive) to **Alt+[ / Alt+]**, because Ctrl+[ acts as Esc in Vim. Keys you have changed yourself stay as they are when you switch Vim mode on or off. Also on Windows and Linux, Vim's Ctrl keys (Ctrl+V, Ctrl+A, Ctrl+X, Ctrl+D and so on) are not Vim commands, so your usual shortcuts such as undo (Ctrl+Z), copy and paste keep working. Only Ctrl+R (redo) and Ctrl+[ (Esc) are Vim commands.

## 6. Working with Claude Code and Codex

```sh
yunagi agent setup
```

registers Yunagi with Claude Code and Codex. Run it in a terminal: it shows what it will write and asks for confirmation (`y`) first, and keeps a copy of your original settings file next to it. It cannot run from a non-interactive shell. Agents start with read-only access. To let them add TODOs or record blockers, enable the permissions in Settings → Agents. Every change made by an agent is marked and can be undone one by one from the change history.

You can also check this from the terminal. Changing permissions and undoing a change show what will happen and ask for confirmation (`y`) first; they cannot run from a non-interactive shell.

```sh
yunagi agent permissions                              # what each agent may do
yunagi agent permissions claude-code --allow todo     # allow a permission (--deny removes one)
yunagi agent history                                  # agent changes, newest first (* = not reviewed yet)
yunagi agent revert <change ID> [--force]             # undo one change
```

Undoing restores only the part that the change modified. If you have changed that same part yourself since, it stops without undoing (check the difference, then add `--force` to overwrite that part). For items that were carried over and TODOs you dropped, a change to their state or text cannot be undone (they keep the state they ended the day with); undoing the addition of such an item, which deletes it, is still possible.

Content that an agent reads from Yunagi is sent to that AI service through the agent, under your own agreement with that service. AI usage is billed by that service; Yunagi's agent integration is free.

You can also use the CLI directly. Writing from the CLI is not allowed at first either; to enable it, turn on the permissions for `cli` in Settings → Agents (permission changes can only be confirmed in the app's Settings or at an interactive terminal prompt).

```sh
yunagi todo add "Reply to review comments"
yunagi board get
git log -1 --format=%s | yunagi reflection append -
```

### Requests for agents

In Board mode, select a TODO and press **Mod+Shift+A** to copy a Markdown request for an agent to the clipboard. It contains the TODO's text and, if you sent the TODO from a line of your note, the notes under the same timestamped heading and the unresolved blockers sent from that section. Paste it straight into the agent's input. The request includes your notes, so check where you paste it.

In a terminal, `yunagi handoff export <TODO ID>` prints the same text (find the ID with `yunagi board get`). An agent with read permission can get it with `export_handoff`.

## 7. Your data

- Yunagi never sends your data anywhere. Your data is stored in the data location on this computer (for content read by agents, see section 6).
- To check for new versions, Yunagi sends only the app version, OS, and architecture, for example at startup (the download host records your IP address and the time). You can turn this off in Settings → General.
- See or change the data location in Settings → Data. Backups of the last 7 days are created automatically.
- If you put the data location in an iCloud Drive or Dropbox folder, open Yunagi on only one computer at a time. To use several computers, we recommend syncing Markdown exports or backups instead.
- You can export a day or a date range as Markdown.

## 8. Uninstall

```sh
yunagi uninstall          # removes the app and CLI, keeps your data
yunagi uninstall --purge  # also removes your data
```

`--purge` removes:

- Your note data (the default data folder; if you changed the data location in Settings, the `yunagi.db` and backups in that location)
- The cache and similar files that Yunagi's display component created (named after `io.github.misty-swan.yunagi`: one folder on Linux and Windows, a few places under `~/Library` on macOS)

The data location folder itself is not removed, because it may hold other files. If Yunagi is running, nothing is removed and the command stops. Quit Yunagi first.

If you installed with Homebrew, `yunagi uninstall` removes nothing and shows the Homebrew command instead: `brew uninstall --cask yunagi` removes the app only, and `brew uninstall --cask --zap yunagi` also removes your data. If you changed the data location in Settings, `--zap` does not remove the data there, so delete it by hand.
