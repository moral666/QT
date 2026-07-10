#!/usr/bin/env bash
# Compila o crate FFI e gera os bindings para Kotlin (Android), Swift (iOS)
# e Python (usado para testar o FFI sem precisar de Android Studio/Xcode).
#
# Uso: ./ffi/generate_bindings.sh
# Requisitos: Rust estavel (rustup) instalado. Ver README.md da raiz.

set -euo pipefail
cd "$(dirname "$0")/.."

echo ">>> A compilar o crate FFI..."
cargo build -p secure_messenger_ffi

LIB_PATH="target/debug/libsecure_messenger_ffi.so"
if [ ! -f "$LIB_PATH" ]; then
    # macOS produz .dylib em vez de .so
    LIB_PATH="target/debug/libsecure_messenger_ffi.dylib"
fi

mkdir -p ffi/bindings

echo ">>> A gerar bindings Python (para testar sem Android/iOS)..."
cargo run -p secure_messenger_ffi --bin uniffi-bindgen -- \
    generate --library "$LIB_PATH" --language python --out-dir ffi/bindings
cp "$LIB_PATH" ffi/bindings/

echo ">>> A gerar bindings Kotlin (Android)..."
cargo run -p secure_messenger_ffi --bin uniffi-bindgen -- \
    generate --library "$LIB_PATH" --language kotlin --out-dir ffi/bindings/kotlin

echo ">>> A gerar bindings Swift (iOS)..."
cargo run -p secure_messenger_ffi --bin uniffi-bindgen -- \
    generate --library "$LIB_PATH" --language swift --out-dir ffi/bindings/swift

echo ">>> A correr o teste Python contra os bindings gerados..."
python3 ffi/tests/test_ffi_bindings.py

echo ">>> Concluido. Bindings em ffi/bindings/{kotlin,swift}/ e ffi/bindings/secure_messenger_ffi.py"
