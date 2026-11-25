#!/usr/bin/env bash
# Builds Starbase's own signed Flatpak repository (a static site) from a .flatpak bundle.
# Flathub is not an option for now (linux/flathub-submission.md), so users add our repository
# instead: `flatpak install --from <url>/starbase.flatpakref`. release.yml runs this on every
# vX.Y.Z tag and publishes the result on GitHub Pages.
#
#   make-repo.sh <bundle.flatpak> <site-dir> <public-url> <key-fingerprint> <public-key.asc>
#
# The signing key must already be in $GNUPGHOME and usable without a prompt (release.yml presets
# its passphrase in gpg-agent). The site gets:
#   repo/                   the OSTree repository: the app on branch `stable`, the commit and the
#                           summary signed, static deltas generated (faster, smaller downloads)
#   starbase.flatpakref     install file: `flatpak install --from <url>/starbase.flatpakref`
#   starbase.flatpakrepo    remote file:  `flatpak remote-add starbase <url>/starbase.flatpakrepo`
#   starbase-release-signing.asc, index.html
# Each release ADDS to the published repository: the current version is fetched from <public-url>
# (signature checked against our key), the new commit is made on top of it, and the repository
# keeps the newest two versions, with a static delta from the previous one, so an update
# downloads only what changed. Starting an empty repository (nothing published yet, HTTP 404)
# needs ALLOW_EMPTY=1, so a misconfigured URL or a vanished site cannot silently reset it.
# PREVIOUS_REPO_URL overrides where the published repository is fetched from (tests).
set -euo pipefail

[ $# -eq 5 ] || { sed -n '2,8p' "$0" >&2; exit 2; }
bundle=$1 site=$2 url=${3%/} key=$4 pubkey=$5
APP_ID=io.github.DomeShifter.Starbase
BRANCH=stable
: "${GNUPGHOME:?set GNUPGHOME to the keyring that holds the signing key}"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# 1. The bundle into a scratch repository, unsigned.
ostree init --mode=archive-z2 --repo="$tmp/src"
flatpak build-import-bundle --no-update-summary "$tmp/src" "$bundle"
src_ref=$(ostree refs --repo="$tmp/src" | grep -m1 "^app/$APP_ID/")
arch=$(cut -d/ -f3 <<< "$src_ref")
echo "bundle ref: $src_ref"

# 2. A signed commit on app/<id>/<arch>/stable in the site's repository, then the signed summary
#    and the static deltas.
rm -rf "$site"
mkdir -p "$site"
ostree init --mode=archive-z2 --repo="$site/repo"
gpg --dearmor < "$pubkey" > "$tmp/key.gpg"
prev=${PREVIOUS_REPO_URL:-$url/repo}
code=$(curl -sS -o /dev/null -w '%{http_code}' "${prev%/}/config" || echo 000)
if [ "$code" = 200 ]; then
  # A failed fetch stops the release: publishing without it would drop users' update path.
  ostree remote add --repo="$site/repo" --gpg-import="$tmp/key.gpg" published "${prev%/}/"
  ostree pull --repo="$site/repo" --mirror --depth=1 published "app/$APP_ID/$arch/$BRANCH"
  ostree remote delete --repo="$site/repo" published
  echo "published version: $(ostree rev-parse --repo="$site/repo" "app/$APP_ID/$arch/$BRANCH")"
elif [ "$code" = 404 ] && [ "${ALLOW_EMPTY:-0}" = 1 ]; then
  echo "nothing published at $prev yet: starting an empty repository (ALLOW_EMPTY=1)"
elif [ "$code" = 404 ]; then
  echo "nothing is published at $prev (HTTP 404). For the very first publish, rerun with ALLOW_EMPTY=1" >&2
  echo "(Release > Run workflow > flatpak_repo_start_empty); otherwise check the URL and Pages." >&2
  exit 1
else
  echo "cannot tell whether $prev exists (HTTP $code): stopping rather than replacing it" >&2
  exit 1
fi
sign=(--gpg-sign="$key" --gpg-homedir="$GNUPGHOME")
flatpak build-commit-from --src-repo="$tmp/src" --src-ref="$src_ref" "${sign[@]}" \
  --no-update-summary "$site/repo" "app/$APP_ID/$arch/$BRANCH"
flatpak build-update-repo "${sign[@]}" --gpg-import="$tmp/key.gpg" \
  --title="Starbase" --comment="Starbase: spreadsheets and documents, familiar to Google Workspace users" \
  --homepage="https://github.com/DomeShifter/Starbase" --default-branch="$BRANCH" \
  --generate-static-deltas --prune --prune-depth=1 "$site/repo"
# The lock file is not part of the repository (and Pages leaves out dotfiles anyway).
rm -f "$site/repo/.lock"

# 3. The install and remote files. GPGKey is the public key, binary, base64 (Flatpak's format).
gpgkey=$(base64 -w0 < "$tmp/key.gpg")
cat > "$site/starbase.flatpakref" <<EOF
[Flatpak Ref]
Name=$APP_ID
Branch=$BRANCH
Title=Starbase
Url=$url/repo/
SuggestRemoteName=starbase
Homepage=https://github.com/DomeShifter/Starbase
IsRuntime=false
RuntimeRepo=https://dl.flathub.org/repo/flathub.flatpakrepo
GPGKey=$gpgkey
EOF
cat > "$site/starbase.flatpakrepo" <<EOF
[Flatpak Repo]
Title=Starbase
Url=$url/repo/
Homepage=https://github.com/DomeShifter/Starbase
Comment=Starbase: spreadsheets and documents, familiar to Google Workspace users
DefaultBranch=$BRANCH
GPGKey=$gpgkey
EOF
cp "$pubkey" "$site/starbase-release-signing.asc"
cat > "$site/index.html" <<EOF
<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Starbase Flatpak repository</title>
<style>body{font:16px/1.5 system-ui,sans-serif;max-width:46rem;margin:2rem auto;padding:0 1rem}pre{background:#8882;padding:.6rem;overflow-x:auto}</style>
</head><body>
<h1>Starbase Flatpak repository</h1>
<p>Starbase Drive, Sheets and Docs for Linux, signed with the
<a href="starbase-release-signing.asc">Starbase release key</a>
(fingerprint <code>$key</code>). Install:</p>
<pre>flatpak install --user --from $url/starbase.flatpakref</pre>
<p>Updates then come with <code>flatpak update</code>. To add only the repository:
<code>flatpak remote-add --user --if-not-exists starbase $url/starbase.flatpakrepo</code>.
The runtime comes from Flathub. Source and more: <a href="https://github.com/DomeShifter/Starbase">github.com/DomeShifter/Starbase</a>.</p>
</body></html>
EOF

# 4. Check: a client that trusts only our key accepts the repository and sees the app.
export FLATPAK_USER_DIR="$tmp/client"
flatpak remote-add --user --gpg-import="$tmp/key.gpg" check "file://$(realpath "$site/repo")"
flatpak remote-ls --user check --columns=application,branch,arch | tee "$tmp/ls"
grep -q "^${APP_ID}[[:space:]]*${BRANCH}" "$tmp/ls"
# Pulling the app checks the commit's signature too (no runtime needed: --no-deps).
flatpak install --user -y --noninteractive --no-deps --no-related check "$APP_ID//$BRANCH"
flatpak info --user "$APP_ID//$BRANCH" | grep -E 'Commit|Parent|Branch|Origin'
# The appstream branch (software centres) is signed too: this pull verifies it.
flatpak update --user --appstream check
ostree log --repo="$site/repo" "app/$APP_ID/$arch/$BRANCH" | grep -E '^commit|Version|Date'
ostree static-delta list --repo="$site/repo"
du -sh "$site"
