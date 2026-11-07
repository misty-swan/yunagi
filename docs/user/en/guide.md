# Yunagi User Guide

Yunagi turns your screen into a single sheet of paper for today's work. It keeps only three things, organized by day: what to do today, what is blocking you, and a short reflection at the end of the day.

In this guide, **Mod** means the ⌘ key on macOS and the Ctrl key on Windows and Linux.

## 1. Install and update

### Install

#### macOS and Linux

```sh
curl -fsSL https://getyunagi.com/install.sh | sh
```

#### Windows (PowerShell)

```powershell
irm https://getyunagi.com/install.ps1 | iex
```

#### Installing safely

If you prefer to read the script first, open the URL, review it, and run the saved file. The install script automatically verifies each download against its published SHA-256 hash. If you download a package manually, verify it with `SHA256SUMS` and its minisign signature `SHA256SUMS.sig` from the official site; the steps are on the site's install page.

#### Where it goes

The install script installs Yunagi for the signed-in user only (no administrator rights needed).

- macOS: `~/Applications/Yunagi.app`. The `yunagi` command is linked at `~/.local/bin/yunagi`.
- Linux: `~/.local/opt/yunagi/` (the AppImage and the command). The command is linked at `~/.local/bin/yunagi`, and Yunagi is added to your application list.
- Windows: `%LOCALAPPDATA%\Programs\Yunagi`. Yunagi is added to the Start menu, and its `bin` folder is added to your user `PATH`.

If `~/.local/bin` is not in your `PATH`, the install script shows how to add it. Your notes are stored apart from the app (see section 7), so reinstalling keeps your data.

### Update

- Yunagi checks for a new version at startup and every 24 hours after that. When there is one, "New version" appears at the bottom of the sidebar (at the right of the top bar when the sidebar is closed).
- Select "New version" to see what changed, then press "Update and restart". Choosing "Update and restart" in the Mod+K search and commands does the same. Yunagi never restarts on its own while you work, and it saves what you are typing before it restarts.
- If an update fails, nothing is replaced and "Couldn't update" appears. You can keep working and try again.
- When an update changes the data format, Yunagi makes a backup before the new version first opens your data. If converting the data fails, it goes back to the backup automatically without changing your data, and shows how to reinstall the previous version.
- To stop the checks, turn off "Check for updates automatically" in the General section of Settings. When it is off, Yunagi does not make this connection at all.
- On Linux, the tar.gz and .deb packages do not update themselves from within the app. To update, install the new version the same way (the AppImage updates automatically).

To install a specific version (for example, to go back to an older one), run the install script with the environment variable `YUNAGI_VERSION` set to that version.

```sh
curl -fsSL https://getyunagi.com/install.sh | YUNAGI_VERSION=1.2.3 sh   # macOS and Linux
```

```powershell
$env:YUNAGI_VERSION="1.2.3"; irm https://getyunagi.com/install.ps1 | iex   # Windows
```

## 2. First launch

Open Yunagi from your applications. After that, press **Mod+Shift+D** (⌘⇧D on macOS, Ctrl+Shift+D on Windows and Linux) to bring it up from anywhere.

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

Yunagi has two modes. Board mode (shown as Board) lays out your TODOs, blockers, and reflection for sorting through. Note mode (Note) gives the whole screen to the day's notes. Press **Mod+J** to switch between them.

1. **Morning**: The first time you open Yunagi on a new day, the TODOs left over from the last day you used it and the blockers that are still open appear on a "CARRY OVER" screen. Everything starts out checked, so uncheck what you won't continue today, then press "Carry over selected", "Carry over all", or "Don’t carry over". TODOs carried over three times or more (you can change the number in Settings) come with "Split" and "Drop" options, a good moment to rethink them. If you don't want to decide yet, press Esc to put it off (it shows once more the next time you bring Yunagi to the front, and you can always open it from "Open carry-over" in Mod+K). Once you confirm, Mod+Z can't undo it.
2. **During the day**: In Board mode, work through your TODOs from the top. The order is the priority. Press Enter at the end of a line to add an item, Mod+Enter to mark it done, and Mod+Backspace to delete it (Mod+Z brings it back). Write down anything that's blocking you under "Blockers". Mod+1, Mod+2 and Mod+3 move between the TODO, Blockers and Reflection areas.
3. **In meetings**: Press **Mod+J** to switch to Note mode and write freely. **Mod+Shift+H** inserts a timestamped heading such as "10:00 Standup".
4. **After meetings**: On a line that became a task, press **Mod+Shift+T** to send it to TODOs. On a line that describes a problem, press **Mod+Shift+B** to send it to Blockers. Press **Mod+J** again to go back to Board mode.
5. **End of day**: Write a short reflection: what went well, what got stuck, and your first move for tomorrow (**Mod+Shift+R** inserts the headings).

Everything is saved automatically. There is no save button. If saving fails, "Not saved" appears in the top bar. What you typed stays on screen and Yunagi keeps retrying.

The meter at the top of the screen shows your day as a sky and a sea. The sun marks the current time (it moves from 9:00 to 19:00 by default; you can change this in the General section of Settings, under Nagi meter hours), the waves show the work that's left, and the path of light on the water shows the work you've finished. When nothing is left, the sea goes flat and the meter reads "NAGI".

The sidebar on the left holds Today (with the number of items left), Archive, Agents (the history of changes made by agents, with the number you haven't reviewed), Recent (the latest days), and Settings. Press **Mod+\\** to open or close it. It closes automatically in Note mode.

To look back at past days, press **Mod+Shift+L** to open Archive. It shows each day of the week (Mon–Fri) along with the "went well" and "got stuck" lines from your recent reflections, and the reflection and note headings of the selected day. Use ← / → to pick a day, **Mod+[ / Mod+]** to move by week, and Enter to open that day. Press Esc (or **Mod+Shift+L**) to go back.

**Mod+K** opens search and commands. You can search the TODOs, blockers, notes, and reflections of every day, or type a date such as "2026-10-31" or "yesterday" to jump to that day. Commands such as opening Settings or changing the layout are here too.

### Display settings

In the Display section of Settings you can choose the theme (match the OS, light, or dark), the font size, the note font (same as body, or monospace), and the layout. The layouts are A (TODOs on the left, Blockers at the upper right, Reflection at the lower right), B (three columns), and C (stacked). While the window is narrower than 900px, C is used whatever you chose. The display language, Japanese or English, is set under Language in the General section.

## 4. Key shortcuts

| Action | Keys |
| --- | --- |
| Bring up Yunagi | Mod+Shift+D |
| Note mode ⇄ Board mode | Mod+J |
| Send line to TODOs / Blockers | Mod+Shift+T / Mod+Shift+B |
| Insert a timestamped heading (Note) | Mod+Shift+H |
| Insert the reflection headings | Mod+Shift+R |
| Move between the TODO, Blockers and Reflection areas | Mod+1 / Mod+2 / Mod+3 |
| Toggle done / resolved | Mod+Enter |
| Delete an item | Mod+Backspace |
| Copy the selected TODO as a request | Mod+Shift+A |
| Reorder items | Alt+↑ / Alt+↓ |
| Previous / next day / today | Mod+[ / Mod+] / Mod+T |
| Undo / redo | Mod+Z / Mod+Shift+Z (Ctrl+Y also works on Windows and Linux) |
| Search and commands | Mod+K |
| Open / close the sidebar | Mod+\\ |
| Archive | Mod+Shift+L |
| Settings | Mod+, |

On macOS, Alt is the Option key. The bottom edge of the screen shows the main keys you can use right now.

Except for undo and redo, the shortcuts listed here can be viewed and changed in the Operation section of Settings, under Keyboard shortcuts. A key can't be shared by two operations that work in the same place, and the keys for undo, redo, copy and paste can't be changed.

## 5. Vim mode

Enable it in the Operation section of Settings, under Vim mode. Notes and reflections support Normal, Insert, and Visual modes. In notes, `<Space>t` sends a line to TODOs and `<Space>b` to Blockers (in Visual mode, the selected lines), and `<Space>h` inserts a timed heading so you can type its title right away. In reflections the `<Space>` keys do nothing. The current mode is shown in small letters at the bottom right of the editor. In Board mode, use j / k to move, o to add, i to edit, x to toggle done/resolved, dd to delete, and u to undo. When you leave Insert mode, your Japanese (or other) input method switches back to direct input automatically (with "Restore input state in Insert mode" in Settings, you choose whether entering Insert mode returns the input method to the state it had when you last left Insert mode). On Linux where neither fcitx5 nor IBus is running, Yunagi can't switch the input method automatically. Settings then shows a notice, and you should switch the input method to English by hand after leaving Insert mode.

On Windows and Linux, enabling Vim mode changes previous/next day (and previous/next week in Archive) to **Alt+[ / Alt+]**, because Ctrl+[ acts as Esc in Vim. Keys you have changed yourself stay as they are when you switch Vim mode on or off. Also on Windows and Linux, Vim's Ctrl keys (Ctrl+V, Ctrl+A, Ctrl+X, Ctrl+D and so on) are not Vim commands, so your usual shortcuts such as undo (Ctrl+Z), copy and paste keep working. Only Ctrl+R (redo) and Ctrl+[ (Esc) are Vim commands.

## 6. Working with Claude Code and Codex

```sh
yunagi agent setup
```

registers Yunagi with Claude Code and Codex. Run it in a terminal: it shows what it will write and asks for confirmation (`y`) first, and keeps a copy of your original settings file next to it. It cannot run from a non-interactive shell. Agents start with read-only access. To let them add TODOs or record blockers, enable the permissions in the Agents section of Settings. Every change made by an agent is marked and can be undone one by one from the change history (Agents in the sidebar).

You can also check this from the terminal. Changing permissions and undoing a change show what will happen and ask for confirmation (`y`) first; they cannot run from a non-interactive shell.

```sh
yunagi agent permissions                              # what each agent may do
yunagi agent permissions claude-code --allow todo     # allow a permission (--deny removes one)
yunagi agent history                                  # agent changes, newest first (* = not reviewed yet)
yunagi agent revert <change ID> [--force]             # undo one change
```

Undoing restores only the part that the change modified. If you have changed that same part yourself since, it stops without undoing (check the difference, then add `--force` to overwrite that part). For items that were carried over and TODOs you dropped, a change to their state or text cannot be undone (they keep the state they ended the day with); undoing the addition of such an item, which deletes it, is still possible.

Content that an agent reads from Yunagi is sent to that AI service through the agent, under your own agreement with that service. AI usage is billed by that service; Yunagi's agent integration is free.

You can also use the CLI directly. Writing from the CLI is not allowed at first either; to enable it, turn on the permissions for `cli` in the Agents section of Settings (permission changes can only be confirmed in the app's Settings or at an interactive terminal prompt).

```sh
yunagi todo add "Reply to review comments"
yunagi board get
git log -1 --format=%s | yunagi reflection append -
```

`yunagi --help` lists all the commands.

### Requests for agents

In Board mode, select a TODO and press **Mod+Shift+A** to copy a Markdown request for an agent to the clipboard. It contains the TODO's text and, if you sent the TODO from a line of your note, the notes under the same timestamped heading and the unresolved blockers sent from that section. Paste it straight into the agent's input. The request includes your notes, so check where you paste it.

In a terminal, `yunagi handoff export <TODO ID>` prints the same text (find the ID with `yunagi board get`). An agent with read permission can get it with `export_handoff`.

## 7. Your data

- Yunagi never sends your data anywhere. Your data is stored in the data location on this computer (for content read by agents, see section 6).
- To check for new versions, Yunagi sends only the app version, OS, and architecture, for example at startup (the download host records your IP address and the time). You can turn this off in the General section of Settings.
- See or change the data location in the Data section of Settings. Backups of the last 7 days are created automatically, and you can restore one from the list in the same section. The backups made before an update and before a restore keep the newest 3 of each.
- If you put the data location in an iCloud Drive or Dropbox folder, open Yunagi on only one computer at a time. To use several computers, we recommend syncing Markdown exports or backups instead.
- You can export a day or a date range as Markdown. In the Data section of Settings, choose the start date, the end date, and a folder, then press Export to get one file per day.

## 8. Uninstall

```sh
yunagi uninstall          # removes the app and CLI, keeps your data
yunagi uninstall --purge  # also removes your data
```

`--purge` removes:

- Your note data (the default data folder; if you changed the data location in Settings, the `yunagi.db` and backups in that location)
- The cache and similar files that Yunagi's display component created (named after `io.github.misty-swan.yunagi`: one folder on Linux and Windows, a few places under `~/Library` on macOS)

The data location folder itself is not removed, because it may hold other files. If Yunagi is running, nothing is removed and the command stops. Quit Yunagi first.

The command lists what it will remove and asks `[y/N]` before it removes anything, so run it in a terminal yourself (it does not run from a script or an AI agent's shell). When it ends, it lists what it removed and anything it could not remove, with the reason (delete those by hand), and exits with a non-zero code in that case. On Windows, the last part of the cleanup (the installed folder and, with `--purge`, the data folder) runs in the background right after the command ends. If something is still there a little later, delete it by hand.

If Yunagi was not installed by the install script (for example, placed by hand, a development build, or a tar.gz), the command shows the location it found and stops without removing anything. It also stops without removing anything if the settings file (`config.json`) is damaged, because the data location is then unknown.
