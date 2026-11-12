# System requirements

## Supported operating systems

| OS | Versions | Architectures |
| --- | --- | --- |
| macOS | The latest 3 releases (Apple Silicon). Intel Macs, up to macOS 26 Tahoe (the last macOS that Apple supports on Intel Macs) | arm64, x86_64 |
| Windows | 10, 11 | x86_64, arm64 |
| Linux | Major distributions. The ones we test on are Ubuntu LTS (GNOME; Wayland and X11) and Fedora (KDE; Wayland) | x86_64, arm64 |

A combination we could not test before a release is marked "trial support (not guaranteed to work)" in the download page and the release notes. **The Intel Mac build (x86_64) is trial support until we can test it on a real Intel Mac: we have not verified it and do not guarantee it works.** Yunagi does not start on macOS earlier than 13.3. The versions we target are those in the table above (Intel Macs are trial support, as noted above).

## Other notes

- You do not need administrator rights. The install script installs for the signed-in user only.
- Every feature works without a network. Yunagi connects only for the update check (on by default; you can turn it off in Settings), for the supporter license key check (only when you enter a key and ask Yunagi to verify it, once each time you ask; entering a key will be available in a later version), and for features you turn on yourself. The update check sends only the version, the OS and the architecture. The key check sends only the key and a fixed identifier for Yunagi's seller account.
- The Linux AppImage needs FUSE 2 to start. On Wayland, the system-wide shortcut may not work. The setup for that is in [chapter 2 of the guide](/en/guide/#2-first-launch).
- The interface is available in Japanese and English. Light and dark follow your OS appearance setting.
- Your notes are saved as files on your PC. You can change where in Settings.
