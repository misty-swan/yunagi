# Security

Yunagi handles your work notes, so its basic stance is to keep your data from leaving your machine. The full description of the protections is in the [security policy](https://github.com/misty-swan/yunagi/blob/main/docs/release/SECURITY.md) in the public repository (in Japanese).

## Reporting a vulnerability

- **Do not write about it in a public issue.**
- Report it in either of these ways.
  - Email: [security@getyunagi.com](mailto:security@getyunagi.com)
  - The "Security" tab of the public repository → "Report a vulnerability" (GitHub's private reporting)
- Include the affected version (`yunagi --version`), your OS, the steps to reproduce, and what happens. Please do not paste real work data such as your notes.
- We aim to acknowledge a report within 3 business days and to give an initial assessment within 10 business days (this is a goal, not a promise).
- After a fix is released, we will credit the reporter if they agree.

## Supported versions

Security fixes are released as patch versions of the latest minor version (for example 1.4.x). They are not released for older minor versions; you will be asked to update.

## How downloads are protected

- The install script compares the SHA-256 hash of the download and stops if it does not match.
- `SHA256SUMS`, which lists the hashes of every download, carries a minisign signature made with the release key. The steps are in [Verify a download](/en/verify/).
- In-app updates apply only packages signed with Yunagi's own key, and only after the signature is verified.
- Yunagi does not use paid OS code signing. Instead, the methods above let you check that a download is the official one.

## Data

- Data is stored only in your own data directory. The directory and the database are created so that only you can read and write them.
- Network traffic is limited to the update check (on by default; you can turn it off in Settings), the supporter license key check, and features you turn on. The update check sends only the version, the OS and the architecture. The key check happens once, when you enter a key and ask Yunagi to verify it, and sends only the key (entering a key will be available in a later version).
- An AI agent can only read by default. Writing is limited to what you allow, and every change is recorded and can be undone one by one.

## Announcements

Security fixes and key revocations are announced in the [news](/en/news/) and as a GitHub Security Advisory in the public repository.
