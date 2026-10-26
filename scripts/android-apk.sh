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
# El manifiesto de nsh 0.2 solo pide INTERNET y no deja agregar permisos: se vuelve a enlazar el mismo
# (con aapt2, como nsh) más el micrófono, para las notas de voz de los hilos, y el ícono de Condx
# (android/res: ícono adaptable en vectores; nsh no trae recursos, así que su resources.arsc se reemplaza).
cat > "$TMP/AndroidManifest.xml" <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="com.example.condxmobile">
    <uses-permission android:name="android.permission.INTERNET" />
    <uses-permission android:name="android.permission.RECORD_AUDIO" />
    <application android:label="Condx"
        android:icon="@mipmap/ic_launcher"
        android:roundIcon="@mipmap/ic_launcher"
        android:extractNativeLibs="false"
        android:enableOnBackInvokedCallback="true"
        android:supportsRtl="true"
        android:theme="@android:style/Theme.Material.Light.NoActionBar">
        <meta-data android:name="com.longbridge.nsh.RUST" android:value="true" />
        <activity android:name="com.longbridge.nsh.AppActivity" android:exported="true"
            android:windowSoftInputMode="adjustResize"
            android:configChanges="orientation|screenSize|screenLayout|smallestScreenSize|keyboard|keyboardHidden|navigation|uiMode|density|fontScale|locale|layoutDirection">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
XML
# La versión sale de Cargo.toml; el número (versionCode) crece con ella: Android solo instala encima
# una versión con número mayor o igual (y la misma firma).
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
IFS=. read -r MA MI PA <<<"$VERSION"
CODE=$((MA * 10000 + MI * 100 + PA))
"$BT/aapt2" compile --dir android/res -o "$TMP/res.zip"
"$BT/aapt2" link -o "$TMP/manifest.apk" "$TMP/res.zip" -I "$ANDROID_HOME/platforms/android-35/android.jar" --manifest "$TMP/AndroidManifest.xml" --min-sdk-version 28 --target-sdk-version 35 --version-code "$CODE" --version-name "$VERSION"
(cd "$TMP" && unzip -q -o manifest.apk -d m && zip -q -d in.apk AndroidManifest.xml resources.arsc && cd m && zip -q -X -0 ../in.apk resources.arsc && zip -q -X -r ../in.apk AndroidManifest.xml res)
"$BT/zipalign" -P 16 -f 4 "$TMP/in.apk" "$TMP/aligned.apk"
"$BT/apksigner" sign --ks "$HOME/.android/debug.keystore" --ks-pass pass:android --key-pass pass:android --out .nsh/out/condx-mobile-arm64.apk "$TMP/aligned.apk"
"$BT/zipalign" -c -P 16 4 .nsh/out/condx-mobile-arm64.apk
echo "✓ .nsh/out/condx-mobile-arm64.apk ($VERSION, $CODE)"
