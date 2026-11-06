# Download and install

Yunagi installs with one line in a terminal. It runs on macOS, Windows and Linux. You do not need administrator rights (it installs for the signed-in user only). The supported systems are listed under [System requirements](/en/requirements/).

## 1. Install from a terminal

### macOS and Linux

```sh
curl -fsSL https://getyunagi.com/install.sh | sh
```

### Windows (PowerShell)

```powershell
irm https://getyunagi.com/install.ps1 | iex
```

The script detects your OS and architecture, downloads the file and its SHA-256 hash, and compares them. If they do not match, it stops without installing anything. At the end it lists where it put things (the locations are also in [chapter 1 of the guide](/en/guide/#1-install-and-update)).

## 2. Read the script before you run it

`curl ... | sh` runs the script without showing it to you. If you want to check it first, save it, read it, and then run the saved file.

macOS and Linux:

```sh
curl -fsSLo install.sh https://getyunagi.com/install.sh
less install.sh
sh install.sh
```

Windows (PowerShell):

```powershell
irm https://getyunagi.com/install.ps1 -OutFile install.ps1
notepad install.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1
```

You can also read the scripts on this site ([install.sh](/install.sh), [install.ps1](/install.ps1)). The source of truth is [`install/install.sh`](https://github.com/misty-swan/yunagi/blob/main/install/install.sh) and [`install/install.ps1`](https://github.com/misty-swan/yunagi/blob/main/install/install.ps1) in the public repository, and the site serves the same content. The scripts are not protected by a signature. If in doubt, compare them with the repository.

Saving and running the script also gives you its exit code, which suits automation (`irm ... | iex` does not return one).

## 3. Install a specific version

Set the environment variable `YUNAGI_VERSION` to install that version. Use it to go back to an older one.

```sh
curl -fsSL https://getyunagi.com/install.sh | YUNAGI_VERSION=1.2.3 sh
```

```powershell
$env:YUNAGI_VERSION="1.2.3"; irm https://getyunagi.com/install.ps1 | iex
```

Without it, the script installs the latest version listed in this site's `latest.json`.

## 4. Download manually

<!-- slot:files -->

Check a file you downloaded by hand with [Verify a download](/en/verify/) before you use it.

- macOS: extract the `.app.tar.gz` and put `Yunagi.app` where you like (for example `~/Applications`). If you downloaded it with a browser, macOS may block the first launch. In that case, remove the download flag (the quarantine attribute) and launch it again. If you put the app in `~/Applications`, the command is `xattr -dr com.apple.quarantine ~/Applications/Yunagi.app`. If you put it somewhere else, use that path instead.
- Linux: make the AppImage executable (`chmod +x`) and run it. It needs FUSE 2 to start.
- Windows: run the `-setup.exe`.

The install script sets up the `yunagi` command link and the `PATH`. If you install by hand, set them up yourself.

## 5. Uninstall

`yunagi uninstall` removes the app and the `yunagi` link. Your data is kept by default. See [chapter 8 of the guide](/en/guide/#8-uninstall) for details.
