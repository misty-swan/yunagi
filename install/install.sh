#!/bin/sh
# Yunagi installer for macOS and Linux.
#
#   curl -fsSL https://<site>/install.sh | sh
#
# What it does: docs/release/07-build-and-distribution.md section 4.1.
# Environment variables (all optional):
#   YUNAGI_VERSION      version to install (default: the "version" in latest.json). "v1.2.3" and "1.2.3" both work.
#   YUNAGI_BASE_URL     where release files are (default: GitHub Releases). Files are read from <base>/v<version>/.
#   YUNAGI_LATEST_URL   where latest.json is (default: the official site).
#
# The whole script is wrapped in main(), which is called on the last line, so a download that is
# cut off halfway never runs a half-written command.
# POSIX sh only (no [[ ]], no pipefail, no local).

set -eu

# These two are placeholders until the public repository and the site are decided (docs/release/07 section 4.1).
DEFAULT_BASE_URL='https://github.com/<public-repository>/releases/download'
DEFAULT_LATEST_URL='https://<site>/latest.json'

APP_ID='io.github.misty-swan.yunagi'

TMP_DIR=''
OS=''
ARCH=''

say() {
  printf '%s\n' "$*"
}

die() {
  printf 'install.sh: %s\n' "$*" >&2
  exit 1
}

cleanup() {
  if [ -n "$TMP_DIR" ] && [ -d "$TMP_DIR" ]; then
    rm -rf "$TMP_DIR"
  fi
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "'$1' is required but was not found."
}

# Sets OS (macos|linux) and ARCH (x86_64|arm64). Unsupported combinations stop here.
detect_platform() {
  uname_s=$(uname -s)
  uname_m=$(uname -m)
  case "$uname_s" in
    Darwin) OS=macos ;;
    Linux) OS=linux ;;
    *) die "unsupported OS: $uname_s (supported: macOS, Linux. On Windows use install.ps1)." ;;
  esac
  case "$uname_m" in
    x86_64 | amd64) ARCH=x86_64 ;;
    arm64 | aarch64) ARCH=arm64 ;;
    *) die "unsupported architecture: $uname_m (supported: x86_64, arm64)." ;;
  esac
  # A shell running under Rosetta on an Apple Silicon Mac says x86_64. Install the native build.
  if [ "$OS" = macos ] && [ "$ARCH" = x86_64 ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then
    ARCH=arm64
  fi
}

# Fails when a URL still holds the placeholder text.
check_url() {
  case "$1" in
    *'<'*) die "the download location is not decided yet ($1). Set $2 to a real URL." ;;
  esac
}

fetch() {
  # fetch URL OUTPUT_FILE
  curl -fsSL --retry 2 -o "$2" "$1" || die "could not download $1"
}

# Prints the version to install, without a leading "v".
resolve_version() {
  version=${YUNAGI_VERSION:-}
  if [ -z "$version" ]; then
    latest_url=${YUNAGI_LATEST_URL:-$DEFAULT_LATEST_URL}
    check_url "$latest_url" YUNAGI_LATEST_URL
    fetch "$latest_url" "$TMP_DIR/latest.json"
    version=$(grep -o '"version"[[:space:]]*:[[:space:]]*"[^"]*"' "$TMP_DIR/latest.json" | head -n 1 | sed 's/.*:[[:space:]]*"\([^"]*\)"/\1/')
    [ -n "$version" ] || die "could not read the version from $latest_url"
  fi
  version=${version#v}
  case "$version" in
    '' | *[!0-9A-Za-z.+-]* | [!0-9]*) die "invalid version: $version" ;;
  esac
  printf '%s\n' "$version"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    die "'sha256sum' or 'shasum' is required but was not found."
  fi
}

# download_verified NAME: downloads NAME and NAME.sha256 into TMP_DIR and stops when the hash differs.
download_verified() {
  name=$1
  base=${YUNAGI_BASE_URL:-$DEFAULT_BASE_URL}
  check_url "$base" YUNAGI_BASE_URL
  fetch "$base/v$VERSION/$name" "$TMP_DIR/$name"
  fetch "$base/v$VERSION/$name.sha256" "$TMP_DIR/$name.sha256"
  expected=$(awk '{print tolower($1); exit}' "$TMP_DIR/$name.sha256")
  case "$expected" in
    *[!0-9a-f]* | '') die "$name.sha256 does not hold a SHA-256 hash." ;;
  esac
  [ "${#expected}" -eq 64 ] || die "$name.sha256 does not hold a SHA-256 hash."
  actual=$(sha256_of "$TMP_DIR/$name")
  if [ "$actual" != "$expected" ]; then
    die "SHA-256 mismatch for $name (expected $expected, got $actual). Nothing was installed."
  fi
  say "Verified $name"
}

# Makes LINK point at TARGET. Refuses to replace a file that is not a link.
make_link() {
  target=$1
  link=$2
  mkdir -p "$(dirname "$link")"
  if [ -e "$link" ] && [ ! -L "$link" ]; then
    die "$link already exists and is not a link. Move it away and run this again."
  fi
  rm -f "$link"
  ln -s "$target" "$link"
}

install_linux() {
  root="$HOME/.local/opt/yunagi"
  appimage="Yunagi_${VERSION}_${ARCH}.AppImage"
  cli="yunagi-cli_${VERSION}_${ARCH}.tar.gz"
  download_verified "$appimage"
  download_verified "$cli"

  # Check what is inside before anything is placed.
  # (Unpack everything so that both "yunagi" and "./yunagi" as the member name work.)
  mkdir -p "$TMP_DIR/cli"
  tar -xzf "$TMP_DIR/$cli" -C "$TMP_DIR/cli" || die "could not unpack $cli"
  [ -f "$TMP_DIR/cli/yunagi" ] || die "$cli does not hold the 'yunagi' command."

  mkdir -p "$root/bin"

  # Write next to the final place and rename, so a running app is never overwritten in place.
  cp "$TMP_DIR/$appimage" "$root/.Yunagi.AppImage.new"
  chmod 755 "$root/.Yunagi.AppImage.new"
  mv -f "$root/.Yunagi.AppImage.new" "$root/Yunagi.AppImage"
  cp "$TMP_DIR/cli/yunagi" "$root/bin/.yunagi.new"
  chmod 755 "$root/bin/.yunagi.new"
  mv -f "$root/bin/.yunagi.new" "$root/bin/yunagi"

  data_home=${XDG_DATA_HOME:-$HOME/.local/share}
  case "$data_home" in
    /*) ;;
    *) data_home="$HOME/.local/share" ;; # XDG says a relative path must be ignored
  esac
  desktop="$data_home/applications/$APP_ID.desktop"
  desktop_note=''
  case "$root" in
    *[\"\\\$\`%]*)
      # These characters need escaping rules in the Exec line that differ between launchers. Skip the entry.
      desktop=''
      desktop_note="No desktop entry was made because $root contains a character that the desktop entry format treats specially (one of \" \\ \$ \` %)."
      ;;
    *)
      mkdir -p "$data_home/applications"
      printf '%s\n' \
        '[Desktop Entry]' \
        'Type=Application' \
        'Name=Yunagi' \
        'Comment=Daily work note for engineers' \
        "Exec=\"$root/Yunagi.AppImage\"" \
        'Terminal=false' \
        'Categories=Office;Utility;' \
        >"$desktop.new"
      mv -f "$desktop.new" "$desktop"
      ;;
  esac

  make_link "$root/bin/yunagi" "$HOME/.local/bin/yunagi"

  say ''
  say "Installed Yunagi $VERSION:"
  say "  app            $root/Yunagi.AppImage"
  say "  command        $root/bin/yunagi"
  say "  link           $HOME/.local/bin/yunagi"
  if [ -n "$desktop" ]; then
    say "  desktop entry  $desktop"
  else
    say "$desktop_note"
  fi
}

install_macos() {
  app="Yunagi_${VERSION}_${ARCH}.app.tar.gz"
  download_verified "$app"

  mkdir -p "$TMP_DIR/stage"
  tar -xzf "$TMP_DIR/$app" -C "$TMP_DIR/stage" || die "could not unpack $app"
  [ -x "$TMP_DIR/stage/Yunagi.app/Contents/MacOS/yunagi" ] || die "$app does not hold Yunagi.app with the 'yunagi' command."

  mkdir -p "$HOME/Applications"
  # Keep the old app aside until the new one is in place, so a failed move does not lose a working install.
  dest="$HOME/Applications/Yunagi.app"
  rm -rf "$dest.old"
  if [ -e "$dest" ]; then
    mv "$dest" "$dest.old" || die "could not replace $dest (is Yunagi running?). Quit it and run this again."
  fi
  if ! mv "$TMP_DIR/stage/Yunagi.app" "$dest"; then
    rm -rf "$dest"
    if [ -e "$dest.old" ]; then
      mv "$dest.old" "$dest"
    fi
    die "could not move Yunagi.app into $HOME/Applications. The previous install was kept."
  fi
  rm -rf "$dest.old"

  make_link "$HOME/Applications/Yunagi.app/Contents/MacOS/yunagi" "$HOME/.local/bin/yunagi"

  say ''
  say "Installed Yunagi $VERSION:"
  say "  app      $HOME/Applications/Yunagi.app"
  say "  command  $HOME/Applications/Yunagi.app/Contents/MacOS/yunagi"
  say "  link     $HOME/.local/bin/yunagi"
}

# True when DIR is one of the entries of PATH.
in_path() {
  case ":${PATH:-}:" in
    *":$1:"* | *":$1/:"*) return 0 ;;
    *) return 1 ;;
  esac
}

path_hint() {
  if in_path "$HOME/.local/bin"; then
    return 0
  fi
  say ''
  say "$HOME/.local/bin is not in your PATH, so 'yunagi' will not be found yet."
  say 'Add this line to your shell startup file (for example ~/.profile or ~/.zshrc), then open a new terminal:'
  say '  export PATH="$HOME/.local/bin:$PATH"'
}

# Same test as the app: docs/design/04-interaction-spec.md (OS-wide shortcut).
is_wayland() {
  session=${XDG_SESSION_TYPE:-}
  if [ "$session" = wayland ]; then
    return 0
  fi
  if [ "$session" != x11 ] && [ -n "${WAYLAND_DISPLAY:-}" ]; then
    return 0
  fi
  return 1
}

wayland_note() {
  if in_path "$HOME/.local/bin"; then
    cmd='yunagi show'
  else
    cmd="$HOME/.local/bin/yunagi show"
  fi
  say ''
  say 'On Wayland the shortcut Mod+Shift+D may not work. If it does not, assign the command'
  say "  $cmd"
  say "to a key of your choice in your desktop environment's keyboard settings:"
  say '  GNOME:  Settings > Keyboard > Keyboard Shortcuts > Custom Shortcuts'
  say '  KDE:    System Settings > Keyboard > Shortcuts > Add Custom Shortcut'
  say '  Hyprland, Sway, etc.: add a key binding (bind) to your config file'
  say "To open the note mode directly, assign another key to '$cmd --note'."
  say "A key binding often does not inherit your shell's PATH. If it says the command is not found,"
  say "write the full path: $HOME/.local/bin/yunagi show"
}

main() {
  need uname
  need curl
  need tar
  need awk
  need grep
  need sed
  detect_platform

  TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/yunagi-install.XXXXXX") || die 'could not create a temporary directory.'
  trap cleanup EXIT
  trap 'exit 1' HUP INT TERM

  [ -n "${HOME:-}" ] || die 'HOME is not set.'
  VERSION=$(resolve_version)
  say "Installing Yunagi $VERSION ($OS, $ARCH)"

  case "$OS" in
    linux) install_linux ;;
    macos) install_macos ;;
  esac

  path_hint
  if [ "$OS" = linux ] && is_wayland; then
    wayland_note
  fi
}

main "$@"
