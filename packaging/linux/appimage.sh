#!/usr/bin/env bash
# Build the Starbase AppImage from already-built native binaries.
#
#   packaging/linux/appimage.sh --starsheets <path> --stardocs <path> [--starslides <path>]
#                               [--starbase-open <path>] [--version <v>]
#
# --starslides adds Starbase Slides (slides/app) and its menu entry; without it the AppImage is
# built without Slides (a warning says so).
# --starbase-open adds the file manager's open handler (starsheets/crates/open) and its hidden
# "Open with" entry; without it the AppImage is built without the handler (a warning says so).
#
# Writes to $DIST (default dist/release):
#   starbase-<version>-linux-<arch>.AppImage
#   starbase-<version>-linux-<arch>.AppImage.zsync   (when zsyncmake is on PATH)
#
# <arch> is this machine's (x86_64 or aarch64). The version defaults to $STARBASE_VERSION, else
# `git describe --tags`, else 0.0.0-dev. Build the binaries on an old distro (CI: ubuntu-22.04,
# glibc 2.35) so they run on newer ones.
#
# Needs: python3, curl, sha256sum. appimagetool and the AppImage runtime are downloaded once
# into $CACHE (default target/appimage-tools) and checked against pinned SHA-256 sums.
# Optional: rsvg-convert or resvg (PNG icons), desktop-file-validate and appstreamcli (checks),
# zsyncmake (delta updates).
#
# Adapted from WordCraft's packaging/linux/package.sh (storytold/wordcraft, MIT OR Apache-2.0).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
ID=io.github.DomeShifter.Starbase

SHEETS_BIN="" DOCS_BIN="" SLIDES_BIN="" OPEN_BIN="" VERSION="${STARBASE_VERSION:-}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --starsheets) SHEETS_BIN="$2"; shift 2 ;;
    --stardocs) DOCS_BIN="$2"; shift 2 ;;
    --starslides) SLIDES_BIN="$2"; shift 2 ;;
    --starbase-open) OPEN_BIN="$2"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    -h | --help) sed -n '2,26p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[[ -x "$SHEETS_BIN" ]] || { echo "error: --starsheets <executable> is required" >&2; exit 2; }
[[ -x "$DOCS_BIN" ]] || { echo "error: --stardocs <executable> is required" >&2; exit 2; }
[[ -z "$OPEN_BIN" || -x "$OPEN_BIN" ]] || { echo "error: --starbase-open needs an executable" >&2; exit 2; }
[[ -z "$SLIDES_BIN" || -x "$SLIDES_BIN" ]] || { echo "error: --starslides needs an executable" >&2; exit 2; }
if [[ -z "$VERSION" ]]; then
  VERSION="$(git -C "$ROOT" describe --tags --abbrev=0 2>/dev/null | sed 's/^v//' || true)"
  VERSION="${VERSION:-0.0.0-dev}"
fi
BUILD_DATE="${STARBASE_BUILD_DATE:-$(date -u +%Y-%m-%d)}"

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) ;;
  aarch64 | arm64) ARCH=aarch64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac

DIST="$(mkdir -p "${DIST:-$ROOT/dist/release}" && cd "${DIST:-$ROOT/dist/release}" && pwd)"
CACHE="$(mkdir -p "${CACHE:-$ROOT/target/appimage-tools}" && cd "${CACHE:-$ROOT/target/appimage-tools}" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
APPDIR="$WORK/Starbase.AppDir"
BASENAME="starbase-$VERSION-linux-$ARCH"
echo "==> Starbase $VERSION AppImage for $ARCH"

# ---- stage the AppDir ---------------------------------------------------------------------------
install -Dm755 "$SHEETS_BIN" "$APPDIR/usr/bin/starsheets"
install -Dm755 "$DOCS_BIN" "$APPDIR/usr/bin/stardocs"
ENTRIES=("$ID" "$ID.Sheets" "$ID.Docs")
# Starbase Slides: a real menu entry, like Docs (an extra one; the metainfo keeps one launchable).
if [[ -n "$SLIDES_BIN" ]]; then
  install -Dm755 "$SLIDES_BIN" "$APPDIR/usr/bin/starslides"
  ENTRIES+=("$ID.Slides")
else
  echo "warning: no --starslides given: this AppImage has no Starbase Slides" >&2
fi
# The open handler: a hidden "Open with" entry (NoDisplay=true), so the metainfo keeps its one
# launchable. Inside the AppImage it starts Sheets or Docs through the AppImage itself ($APPIMAGE).
if [[ -n "$OPEN_BIN" ]]; then
  install -Dm755 "$OPEN_BIN" "$APPDIR/usr/bin/starbase-open"
  ENTRIES+=("$ID.Open")
else
  echo "warning: no --starbase-open given: this AppImage has no file-manager open handler" >&2
fi
strip "$APPDIR"/usr/bin/* 2>/dev/null || true
for entry in "${ENTRIES[@]}"; do
  install -Dm644 "$HERE/$entry.desktop" "$APPDIR/usr/share/applications/$entry.desktop"
done
install -Dm644 "$HERE/$ID.mime.xml" "$APPDIR/usr/share/mime/packages/$ID.xml"
mkdir -p "$APPDIR/usr/share/metainfo"
# The AppImage is built on Ubuntu 22.04, whose appstreamcli (0.15) rejects the newer
# vcs-browser and contribute URL types that Flathub asks for, so the AppImage copy drops them.
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$BUILD_DATE/g" \
  -e '/<url type="vcs-browser">/d' -e '/<url type="contribute">/d' \
  "$HERE/$ID.metainfo.xml.in" >"$APPDIR/usr/share/metainfo/$ID.metainfo.xml"
"$HERE/icons.sh" "$APPDIR/usr/share/icons"
# The package version, for `Starbase.AppImage --version` (AppRun).
install -Dm644 /dev/null "$APPDIR/usr/share/starbase/version"
printf '%s\n' "$VERSION" >"$APPDIR/usr/share/starbase/version"
mkdir -p "$APPDIR/usr/share/doc/starbase"
for f in README.md LICENSE-MIT LICENSE-APACHE NOTICE; do
  if [[ -f "$ROOT/$f" ]]; then cp "$ROOT/$f" "$APPDIR/usr/share/doc/starbase/"; fi
done

# AppImage root: the dispatcher, the Starbase Drive menu entry (the one desktop integration tools
# such as Gear Lever install) and its icon.
install -Dm755 "$HERE/AppRun" "$APPDIR/AppRun"
cp "$HERE/$ID.desktop" "$APPDIR/$ID.desktop"
if [[ -f "$APPDIR/usr/share/icons/hicolor/256x256/apps/$ID.png" ]]; then
  cp "$APPDIR/usr/share/icons/hicolor/256x256/apps/$ID.png" "$APPDIR/$ID.png"
  ln -s "$ID.png" "$APPDIR/.DirIcon"
else
  cp "$APPDIR/usr/share/icons/hicolor/scalable/apps/$ID.svg" "$APPDIR/$ID.svg"
  ln -s "$ID.svg" "$APPDIR/.DirIcon"
fi

if command -v desktop-file-validate >/dev/null; then
  for f in "$APPDIR"/usr/share/applications/*.desktop; do desktop-file-validate "$f"; done
fi
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --explain "$APPDIR/usr/share/metainfo/$ID.metainfo.xml"
fi

# ---- pinned tools (same versions and sums as WordCraft's package.sh) ---------------------------
APPIMAGETOOL_VERSION=1.9.1
RUNTIME_VERSION=20251108
case "$ARCH" in
  x86_64)
    APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
    RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
    ;;
  aarch64)
    APPIMAGETOOL_SHA256=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
    RUNTIME_SHA256=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444
    ;;
esac
# Downloads $1 to $3 once and checks it against SHA-256 $2, the cached copy again on every run.
fetch_pinned() {
  local url="$1" expected_sha="$2" target="$3"
  if [[ ! -x "$target" ]]; then
    curl --proto '=https' -fsSL -o "$target.download" "$url"
    printf '%s  %s\n' "$expected_sha" "$target.download" | sha256sum -c -
    chmod 755 "$target.download"
    mv "$target.download" "$target"
  fi
  printf '%s  %s\n' "$expected_sha" "$target" | sha256sum -c -
  return 0
}
TOOL="${APPIMAGETOOL:-}"
if [[ -z "$TOOL" ]]; then
  TOOL="$CACHE/appimagetool-$APPIMAGETOOL_VERSION-$ARCH.AppImage"
  fetch_pinned "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-$ARCH.AppImage" \
    "$APPIMAGETOOL_SHA256" "$TOOL"
fi
RUNTIME="$CACHE/appimage-runtime-$RUNTIME_VERSION-$ARCH"
fetch_pinned "https://github.com/AppImage/type2-runtime/releases/download/$RUNTIME_VERSION/runtime-$ARCH" \
  "$RUNTIME_SHA256" "$RUNTIME"

# ---- build --------------------------------------------------------------------------------------
OUT="$DIST/$BASENAME.AppImage"
rm -f "$OUT" "$OUT.zsync"
# Update information: AppImageUpdate, Gear Lever and the like read it from the file and fetch
# only the changed blocks of the newest non-pre-release on GitHub Releases, through the .zsync
# published next to each AppImage. A fork's CI builds point at the fork through GITHUB_REPOSITORY.
REPO="${GITHUB_REPOSITORY:-DomeShifter/Starbase}"
UPDATE_INFO="gh-releases-zsync|${REPO%%/*}|${REPO#*/}|latest|starbase-*-linux-$ARCH.AppImage.zsync"
# Extract-and-run works without FUSE (containers, CI); the pinned static runtime means users
# don't need libfuse2. appimagetool writes the .zsync into its working directory, hence the cd.
(cd "$DIST" && ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream \
  --runtime-file "$RUNTIME" -u "$UPDATE_INFO" "$APPDIR" "$OUT")
echo "wrote $OUT"
if [[ -s "$OUT.zsync" ]]; then
  echo "wrote $OUT.zsync"
else
  echo "warning: zsyncmake not found, so no .zsync was written; delta updates need it" >&2
fi
