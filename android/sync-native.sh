#!/usr/bin/env bash
# Copia os .so (cargo-ndk) e os bindings Kotlin (UniFFI) do projeto Rust para esta app.
# Uso: ./sync-native.sh [caminho/para/QT]   (omissao: ../QT)
set -euo pipefail
QT="${1:-..}"
cd "$(dirname "$0")"
[ -d "$QT/android-out/jniLibs" ] || { echo "Falta $QT/android-out/jniLibs (corre o cargo ndk primeiro)"; exit 1; }
[ -f "$QT/ffi/bindings/kotlin/uniffi/qt_ffi/qt_ffi.kt" ] || { echo "Faltam os bindings (corre $QT/ffi/generate_bindings.sh)"; exit 1; }
rm -rf app/src/main/jniLibs app/src/main/java/uniffi
mkdir -p app/src/main/jniLibs app/src/main/java
cp -r "$QT/android-out/jniLibs/." app/src/main/jniLibs/
cp -r "$QT/ffi/bindings/kotlin/uniffi" app/src/main/java/
echo "OK:"; find app/src/main/jniLibs -name '*.so' -exec ls -lh {} \;
