#!/bin/sh
# Installs the latest fob for Linux or macOS into ${FOB_INSTALL_DIR:-$HOME/.local/bin}.
set -eu

base="https://github.com/fobauth/cli/releases/latest/download"

fail() {
  echo "install.sh: $1" >&2
  exit 1
}

fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -fsSL -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$2" "$1"
  else
    fail "curl or wget is needed to download fob."
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d ' ' -f 1
  else
    fail "sha256sum or shasum is needed to check the download."
  fi
}

# Everything runs from here, so a script cut short in transit does nothing.
main() {
  case "$(uname -s)" in
    Linux) os=unknown-linux-musl ;;
    Darwin) os=apple-darwin ;;
    *) fail "there is no fob for $(uname -s). On Windows, use install.ps1." ;;
  esac
  case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    arm64 | aarch64) arch=aarch64 ;;
    *) fail "there is no fob for $(uname -m)." ;;
  esac
  archive="fob-$arch-$os.tar.gz"
  dir="${FOB_INSTALL_DIR:-$HOME/.local/bin}"

  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  trap 'exit 1' HUP INT TERM

  fetch "$base/$archive" "$tmp/$archive" || fail "couldn't download $base/$archive."
  fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || fail "couldn't download $base/SHA256SUMS."

  expected=$(awk -v name="$archive" '$2 == name || $2 == "*" name { print $1 }' "$tmp/SHA256SUMS")
  [ -n "$expected" ] || fail "SHA256SUMS doesn't list $archive."
  actual=$(sha256 "$tmp/$archive")
  [ -n "$actual" ] || fail "couldn't work out the checksum of $archive."
  [ "$actual" = "$expected" ] || fail "$archive doesn't match its checksum. Nothing was installed."

  tar -xzf "$tmp/$archive" -C "$tmp" fob
  mkdir -p "$dir"
  # Copied beside the old one and moved over it, so a running fob isn't written into.
  cp "$tmp/fob" "$dir/fob.new"
  chmod 755 "$dir/fob.new"
  mv -f "$dir/fob.new" "$dir/fob"

  echo "Installed $("$dir/fob" --version) to $dir/fob"
  case ":$PATH:" in
    *":$dir:"*) ;;
    *) echo "$dir isn't on your PATH. Add it to run fob by name: export PATH=\"$dir:\$PATH\"" ;;
  esac
}

main
