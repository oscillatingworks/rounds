#!/bin/sh
# Installs rounds into a git project in one step:
#
#   curl -fsSL https://raw.githubusercontent.com/oscillatingworks/rounds/main/install.sh | sh -s -- [PROJECT] [options]
#
# On Linux (x86_64 or arm64) it downloads the release binary, checks its
# SHA-256, and runs `rounds install PROJECT [options]` (see `rounds help`). PROJECT
# defaults to the current directory. Without a release binary for the platform (macOS for now), it
# builds the binary with cargo when cargo is there. Set ROUNDS_VERSION to pick a release tag (default: the latest).
set -eu

repo="oscillatingworks/rounds"
version="${ROUNDS_VERSION:-latest}"

fail() {
  echo "rounds: $*" >&2
  exit 1
}

case "$(uname -s)" in
  Linux) os="unknown-linux-musl" ;;
  Darwin) os="apple-darwin" ;;
  *) fail "unsupported system $(uname -s): Linux and macOS are supported" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch="x86_64" ;;
  arm64 | aarch64) arch="aarch64" ;;
  *) fail "unsupported machine $(uname -m)" ;;
esac
target="$arch-$os"

work="$(mktemp -d "${TMPDIR:-/tmp}/rounds.XXXXXX")"
trap 'rm -rf "$work"' EXIT INT TERM

fetch() {
  if command -v curl > /dev/null 2>&1; then
    curl -fsSL "$1" -o "$2"
  elif command -v wget > /dev/null 2>&1; then
    wget -q "$1" -O "$2"
  else
    fail "curl or wget is needed"
  fi
}

# The latest tag from the API: GitHub's releases/latest/download address can serve the previous
# release for some minutes after a new one is published.
if [ "$version" = "latest" ] && fetch "https://api.github.com/repos/$repo/releases/latest" "$work/latest.json" 2> /dev/null; then
  tag="$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$work/latest.json" | head -n 1)"
  [ -n "$tag" ] && version="$tag"
fi
if [ "$version" = "latest" ]; then
  base="https://github.com/$repo/releases/latest/download"
else
  base="https://github.com/$repo/releases/download/$version"
fi
asset="rounds-$target.tar.gz"

if fetch "$base/$asset" "$work/$asset" 2> /dev/null && fetch "$base/$asset.sha256" "$work/$asset.sha256" 2> /dev/null; then
  expected="$(cut -d ' ' -f 1 < "$work/$asset.sha256")"
  if command -v sha256sum > /dev/null 2>&1; then
    actual="$(sha256sum "$work/$asset" | cut -d ' ' -f 1)"
  else
    actual="$(shasum -a 256 "$work/$asset" | cut -d ' ' -f 1)"
  fi
  [ "$expected" = "$actual" ] || fail "checksum mismatch for $asset"
  tar -xzf "$work/$asset" -C "$work"
  binary="$work/rounds"
elif command -v cargo > /dev/null 2>&1; then
  echo "rounds: no release binary for $target; building it with cargo" >&2
  if [ "$version" = "latest" ]; then
    cargo install --quiet --git "https://github.com/$repo" --root "$work" rounds
  else
    cargo install --quiet --git "https://github.com/$repo" --tag "$version" --root "$work" rounds
  fi
  binary="$work/bin/rounds"
else
  fail "no release binary for $target and no cargo to build one (https://rustup.rs)"
fi

"$binary" install "$@"
