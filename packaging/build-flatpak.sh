#!/usr/bin/env bash
#
# Build and install the Brain Flatpak.
#
#   packaging/build-flatpak.sh            build and install --user
#   packaging/build-flatpak.sh --bundle   also write dist/brain.flatpak
#
# The vault is reached through the file portal rather than a --filesystem
# permission; see the note at the top of packaging/flatpak/us.hagreli.Brain.yml.
#
set -euo pipefail

APP_ID="us.hagreli.Brain"
RUNTIME_VERSION="50"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$here"

MANIFEST="packaging/flatpak/$APP_ID.yml"
SOURCES="packaging/flatpak/cargo-sources.json"
BUILD_DIR="$here/.flatpak-build"
DIST="$here/dist"

say()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

command -v flatpak >/dev/null || die "flatpak is not installed"
command -v flatpak-builder >/dev/null \
  || die "flatpak-builder is not installed (sudo apt install flatpak-builder)"

# ---- runtimes -------------------------------------------------------------

# The rust-stable extension is versioned by the *freedesktop* base the GNOME
# SDK is built on, not by the GNOME version — GNOME 50 sits on freedesktop
# 25.08. Read it from the SDK rather than hardcoding a number that silently
# rots into a failed build one release later.
BASE_VERSION="$(flatpak remote-info --show-metadata flathub "org.gnome.Sdk//$RUNTIME_VERSION" 2>/dev/null \
  | sed -n 's/^version = \([0-9][0-9]\.[0-9][0-9]\)$/\1/p' | head -1)"
BASE_VERSION="${BASE_VERSION:-25.08}"

say "Checking runtimes (GNOME $RUNTIME_VERSION on freedesktop $BASE_VERSION)"
for ref in \
  "org.gnome.Platform//$RUNTIME_VERSION" \
  "org.gnome.Sdk//$RUNTIME_VERSION" \
  "org.freedesktop.Sdk.Extension.rust-stable//$BASE_VERSION"
do
  if flatpak info "$ref" >/dev/null 2>&1; then
    echo "  have $ref"
  else
    echo "  installing $ref"
    flatpak install --user -y flathub "$ref"
  fi
done

# ---- vendored cargo dependencies ------------------------------------------
#
# Flathub builds offline, so every crate has to be declared as a source with a
# checksum. flatpak-cargo-generator turns Cargo.lock into exactly that.

if [[ ! -f "$SOURCES" || Cargo.lock -nt "$SOURCES" ]]; then
  say "Generating cargo sources from Cargo.lock"
  GENERATOR="$BUILD_DIR/flatpak-cargo-generator.py"
  mkdir -p "$BUILD_DIR"

  if [[ ! -f "$GENERATOR" ]]; then
    echo "  fetching flatpak-cargo-generator.py from flatpak-builder-tools"
    curl -fsSL -o "$GENERATOR" \
      https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py \
      || die "could not download the generator; fetch it manually into $GENERATOR"
  fi

  # flatpak-cargo-generator is a third-party script with its own dependencies
  # (aiohttp and tomlkit — it moved off `toml`, so having that installed is not
  # enough). Prefer uv, which resolves them into a throwaway environment: no
  # sudo, no venv to maintain, and nothing added to the system interpreter.
  if command -v uv >/dev/null; then
    echo "  running the generator via uv"
    uv run --no-project --quiet \
      --with aiohttp --with tomlkit \
      -- python "$GENERATOR" Cargo.lock -o "$SOURCES"
  else
    # Without uv, fall back to the system interpreter and apt-installed deps.
    PY=/usr/bin/python3
    [[ -x "$PY" ]] || PY=python3

    MISSING=""
    for module in aiohttp tomlkit; do
      "$PY" -c "import $module" 2>/dev/null || MISSING="$MISSING python3-${module}"
    done
    if [[ -n "$MISSING" ]]; then
      die "flatpak-cargo-generator needs modules $PY cannot import.
       Install uv, or:  sudo apt install$MISSING"
    fi
    "$PY" "$GENERATOR" Cargo.lock -o "$SOURCES"
  fi

  COUNT="$(/usr/bin/python3 -c "import json;print(len(json.load(open('$SOURCES'))))" 2>/dev/null || echo '?')"
  echo "  wrote $SOURCES ($COUNT sources)"
else
  say "Cargo sources up to date"
fi

# ---- build ----------------------------------------------------------------

say "Building"
flatpak-builder \
  --user --install --force-clean \
  --state-dir "$BUILD_DIR/state" \
  "$BUILD_DIR/repo" \
  "$MANIFEST"

echo
say "Installed as $APP_ID"
echo "  Run with:  flatpak run $APP_ID"

if [[ "${1:-}" == "--bundle" ]]; then
  mkdir -p "$DIST"
  say "Writing a single-file bundle"
  flatpak build-bundle "$BUILD_DIR/state/repo" "$DIST/brain.flatpak" "$APP_ID" \
    --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo
  echo "  $DIST/brain.flatpak"
fi

cat <<EOF

Run it with:  flatpak run $APP_ID

On first launch Brain asks for a folder to keep notes in. That choice goes
through the file portal, so the sandbox gets access to that folder and nothing
else — and keeps it across restarts.
EOF
