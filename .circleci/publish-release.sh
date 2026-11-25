#!/usr/bin/env bash
set -euo pipefail

: "${GH_TOKEN:?Set GH_TOKEN in CircleCI with Contents: Read and write for this repository}"
: "${CIRCLE_TAG:?Release publishing requires a version tag}"
: "${CIRCLE_PROJECT_USERNAME:?Missing repository owner}"
: "${CIRCLE_PROJECT_REPONAME:?Missing repository name}"
[[ "$CIRCLE_TAG" =~ ^v[0-9] ]] || { echo 'Expected a v-prefixed version tag' >&2; exit 1; }

cd "${1:?Expected the artifacts directory}"
# Refuse to publish a partial release if any workspace package is missing.
packages=(
  opencl_vanity_gpg-x86_64-unknown-linux-gnu.tar.gz
  opencl_vanity_gpg-aarch64-apple-darwin.tar.gz
  opencl_vanity_gpg-x86_64-pc-windows-msvc.zip
)
for package in "${packages[@]}"; do
  test -s "$package"
  test -s "$package.sha256"
done
# Accept older Windows artifacts whose checksum files were written with CRLF.
cat "${packages[0]}.sha256" "${packages[1]}.sha256" "${packages[2]}.sha256" | tr -d '\r' > SHA256SUMS
sha256sum --check SHA256SUMS

repository="$CIRCLE_PROJECT_USERNAME/$CIRCLE_PROJECT_REPONAME"
if gh release view "$CIRCLE_TAG" --repo "$repository" >/dev/null 2>&1; then
  # A rerun only replaces this pipeline's assets; preserve existing release notes.
  gh release upload "$CIRCLE_TAG" "${packages[@]}" SHA256SUMS --clobber --repo "$repository"
else
  flags=(--verify-tag --generate-notes)
  if [[ "$CIRCLE_TAG" == *-* ]]; then
    flags+=(--prerelease)
  fi
  gh release create "$CIRCLE_TAG" "${packages[@]}" SHA256SUMS \
    --repo "$repository" --title "$CIRCLE_TAG" "${flags[@]}"
fi
