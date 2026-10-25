#!/usr/bin/env bash
# Empaqueta para el simulador de iOS con el entitlement del Keychain, instala y abre.
# nsh 0.2 empaqueta sin entitlements y `nsh.json` no permite declararlos: sin
# `keychain-access-groups`, el Keychain responde errSecMissingEntitlement. Volver a firmar el .app
# con `codesign --entitlements` no sirve (el simulador no lo abre: código 163). Lo que hace Xcode es
# meterlos en el binario al enlazar (secciones __entitlements y __ents_der); aquí, igual, por
# RUSTFLAGS. En un dispositivo vienen del perfil de aprovisionamiento
# (`nsh package --target ios --sign … --profile …`).
set -euo pipefail
cd "$(dirname "$0")/.."
ID=$(python3 -c "import json;print(json.load(open('nsh.json'))['id'])")
# Carpeta fija: si la ruta cambiara, RUSTFLAGS cambia y cargo recompila todo.
DIR="$PWD/.nsh/entitlements"
mkdir -p "$DIR"
cat > "$DIR/ent.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>application-identifier</key><string>SIMULATOR.$ID</string>
  <key>keychain-access-groups</key><array><string>SIMULATOR.$ID</string></array>
</dict></plist>
PLIST
xcrun derq query -f xml -i "$DIR/ent.plist" -o "$DIR/ent.der" --raw
export CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUSTFLAGS="-C link-arg=-Wl,-sectcreate,__TEXT,__entitlements,$DIR/ent.plist -C link-arg=-Wl,-sectcreate,__TEXT,__ents_der,$DIR/ent.der"
nsh package --target ios-simulator
APP=.nsh/out/condx-mobile.app
xcrun simctl install booted "$APP"
xcrun simctl launch booted "$ID"
