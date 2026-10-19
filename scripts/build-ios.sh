#!/usr/bin/env bash
# Compila o nucleo Rust (libqt_ffi) para iOS e prepara tudo o que o Xcode precisa:
#
#   Rust -> libqt_ffi.a (iPhone arm64 + simulador arm64/x86_64)
#        -> ios/Frameworks/QtFFI.xcframework   (biblioteca + cabecalhos C)
#        -> ios/Generated/qt_ffi.swift          (bindings Swift gerados pelo UniFFI)
#
# E o MESMO codigo Rust da app Android (criptografia, SQLCipher, historico, protocolo).
#
# Requisitos (num Mac):
#   * Xcode (com as Command Line Tools: xcode-select --install)
#   * Rust >= 1.88 via rustup            https://rustup.rs  (rustup update)
#   * perl + make (vem com o macOS; usados para compilar o OpenSSL/SQLCipher embebidos)
#
# Uso:  ./scripts/build-ios.sh
#
# Usado num Mac para a app iPhone do teste de campo (docs/field-tests.md).
# Se o build falhar, o erro do cargo/xcodebuild diz qual o passo.

set -euo pipefail
cd "$(dirname "$0")/.."

[ "$(uname -s)" = "Darwin" ] || { echo "Este script tem de correr num Mac (o SDK do iOS so existe no macOS)."; exit 1; }
command -v xcodebuild >/dev/null || { echo "Falta o Xcode (xcodebuild)."; exit 1; }
# O nucleo usa `as_chunks` (Rust 1.88+). Com um Rust mais antigo o erro do cargo e confuso.
RUST_MINOR=$(rustc --version | sed -E 's/^rustc 1\.([0-9]+).*/\1/')
[ "$RUST_MINOR" -ge 88 ] || { echo "Rust $(rustc --version | cut -d' ' -f2) e antigo: corre 'rustup update' (precisa de 1.88+)."; exit 1; }

export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-17.0}"
FEATURES="--no-default-features --features ios"
OUT=ios/Frameworks
GEN=ios/Generated
TMP=target/ios-pack

echo ">>> targets Rust para iOS"
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios

echo ">>> cargo build (release) para iPhone e simulador"
for t in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
  echo "    $t"
  cargo build --release -p qt_ffi $FEATURES --target "$t"
done

echo ">>> bindings Swift (a partir de uma build do Mac NAO stripped: o UniFFI le os metadados dela)"
cargo build -p qt_ffi $FEATURES
rm -rf "$TMP" && mkdir -p "$TMP/headers" "$TMP/sim" "$GEN" "$OUT"
cargo run -q -p qt_ffi $FEATURES --bin uniffi-bindgen -- \
  generate --library target/debug/libqt_ffi.dylib --language swift --out-dir "$TMP/swift"
cp "$TMP/swift/qt_ffi.swift" "$GEN/qt_ffi.swift"
cp "$TMP/swift/qt_ffiFFI.h" "$TMP/headers/"
# O Xcode procura "module.modulemap" dentro da pasta de cabecalhos da XCFramework.
cp "$TMP/swift/qt_ffiFFI.modulemap" "$TMP/headers/module.modulemap"

# Garantia: nenhuma funcao "legacy" (que exporia chaves privadas) nos bindings da app.
for sym in generateDhKeypair generateSigningKeypair x3dhInitiate ratchetEncrypt ratchetDecrypt; do
  if grep -q "$sym" "$GEN/qt_ffi.swift"; then echo "ERRO: API legacy '$sym' nos bindings Swift"; exit 1; fi
done

echo ">>> biblioteca unica para o simulador (arm64 + x86_64)"
lipo -create \
  target/aarch64-apple-ios-sim/release/libqt_ffi.a \
  target/x86_64-apple-ios/release/libqt_ffi.a \
  -output "$TMP/sim/libqt_ffi.a"

echo ">>> XCFramework"
rm -rf "$OUT/QtFFI.xcframework"
xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/release/libqt_ffi.a -headers "$TMP/headers" \
  -library "$TMP/sim/libqt_ffi.a" -headers "$TMP/headers" \
  -output "$OUT/QtFFI.xcframework"

echo ">>> verificacao"
ls -lh target/aarch64-apple-ios/release/libqt_ffi.a "$TMP/sim/libqt_ffi.a" | awk '{print "  "$9" ("$5")"}'
echo "  $GEN/qt_ffi.swift ($(wc -l < "$GEN/qt_ffi.swift") linhas)"
echo ">>> OK. Agora: cd ios && xcodegen && open SealerMessenger.xcodeproj"
