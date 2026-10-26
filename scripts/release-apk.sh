#!/usr/bin/env bash
# Una versión nueva de la app del teléfono desde esta Mac (local primero): sube la versión, compila la
# APK firmada con la llave de esta Mac (la misma de la instalada: Android la acepta encima), la respalda
# en GitHub (commit + tag) y deja la ruta para que el daemon la publique en Condx Cloud.
#
#   scripts/release-apk.sh 0.3.0     # o sin versión: la siguiente de parche
# Imprime al final: APK=<ruta> VERSION=<x.y.z> CODE=<n>
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
CURRENT="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
VERSION="${1:-}"
VERSION="${VERSION#v}"
if [ -z "$VERSION" ]; then
  IFS=. read -r MA MI PA <<<"$CURRENT"
  VERSION="$MA.$MI.$((PA + 1))"
fi
TAG="v$VERSION"
[ "$(uname -s)" = "Darwin" ] || { echo "release-apk.sh needs this Mac's Android SDK and signing key" >&2; exit 2; }
[ -f "$HOME/.android/debug.keystore" ] || { echo "no signing key (~/.android/debug.keystore)" >&2; exit 2; }
[ -z "$(git status --porcelain)" ] || { echo "condx-mobile has uncommitted changes: commit them first" >&2; exit 2; }
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then echo "tag $TAG already exists" >&2; exit 2; fi
sed -i '' "s/^version = \"$CURRENT\"/version = \"$VERSION\"/" Cargo.toml
if ! OUT="$(scripts/android-apk.sh 2>&1)"; then echo "$OUT" | tail -30 >&2; git checkout -- Cargo.toml Cargo.lock; exit 1; fi
cargo update -w -q 2>/dev/null || true
git add Cargo.toml Cargo.lock
git commit -q -m "release: $TAG"
git tag -a "$TAG" -m "condx-mobile $TAG"
git -c credential.helper= -c credential.helper='!gh auth git-credential' push -q origin HEAD "$TAG"
IFS=. read -r MA MI PA <<<"$VERSION"
echo "APK=$ROOT/.nsh/out/condx-mobile-arm64.apk VERSION=$VERSION CODE=$((MA * 10000 + MI * 100 + PA))"
