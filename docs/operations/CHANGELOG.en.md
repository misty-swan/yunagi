# Change log

This file follows the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format, and version numbers follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- A daily board (TODOs, blockers, and a reflection)
- Note mode and sending lines to the board
- Carrying over unfinished TODOs and unresolved blockers
- Autosave and automatic backups
- Full-text search
- Vim mode
- Bring Yunagi to the front from any app with a shortcut (Mod+Shift+D by default; it may not work on Wayland)
- The command `yunagi show [--note]` brings Yunagi to the front (it starts it if it is not running; you can call it from your keyboard settings when the shortcut does not work on Wayland)
- AI agent integration through the CLI and an MCP server
- Copy selected TODOs as a request, with the original section of the note and the unresolved blockers attached (Mod+Shift+A; also available as `yunagi handoff export` in the CLI and `export_handoff` in MCP)
- Japanese and English UI

<!--
Rules (this file mirrors CHANGELOG.md, which is the Japanese original; update both together)
- Sections: Added, Changed, Deprecated, Removed, Fixed, Security.
- Write in words a user can read. Do not write about internal implementation.
- Keep the comparison link out until the first release (see CHANGELOG.md). Add it at the first release.
-->
