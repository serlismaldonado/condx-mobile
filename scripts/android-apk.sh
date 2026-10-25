#!/usr/bin/env bash
# APK para instalar a mano en un teléfono Android: solo arm64 (la mitad de tamaño), con páginas de
# 16 KB (Android 15+ avisa si la librería no está alineada) y firmada con la llave de depuración.
# nsh 0.2 no deja elegir ABIs ni flags del enlazador: se pasan por RUSTFLAGS y se quita x86_64
# del APK después. Sale en .nsh/out/condx-mobile-arm64.apk.
set -euo pipefail
cd "$(dirname "$0")/.."
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/27.2.12479018}"
export JAVA_HOME="${JAVA_HOME:-/Applications/Android Studio.app/Contents/jbr/Contents/Home}"
export PATH="$JAVA_HOME/bin:$PATH"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
nsh package --target android
BT="$ANDROID_HOME/build-tools/$(ls "$ANDROID_HOME/build-tools" | sort -V | tail -1)"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
cp .nsh/out/condx-mobile.apk "$TMP/in.apk"
zip -q -d "$TMP/in.apk" 'lib/x86_64/*' 'META-INF/*'
"$BT/zipalign" -P 16 -f 4 "$TMP/in.apk" "$TMP/aligned.apk"
"$BT/apksigner" sign --ks "$HOME/.android/debug.keystore" --ks-pass pass:android --key-pass pass:android --out .nsh/out/condx-mobile-arm64.apk "$TMP/aligned.apk"
"$BT/zipalign" -c -P 16 4 .nsh/out/condx-mobile-arm64.apk
echo "✓ .nsh/out/condx-mobile-arm64.apk"
