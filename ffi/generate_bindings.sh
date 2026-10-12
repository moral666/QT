#!/usr/bin/env bash
# Gera os bindings do crate FFI.
#
#  * Kotlin (Android) e Swift (iOS): a partir do PERFIL DE PRODUCAO
#    (`--features pq`, SEM `legacy-api`). Nao incluem nenhuma funcao que faca
#    chaves privadas ou estados de ratchet atravessar a FFI - o script
#    verifica isto e FALHA se alguma aparecer.
#  * Python: a partir de uma build com `--features pq,legacy-api`, so para
#    testes locais sem Android Studio/Xcode.
#
# Uso: ./ffi/generate_bindings.sh        (requer Rust >= 1.81)

set -euo pipefail
cd "$(dirname "$0")/.."

lib_path() {
    if [ -f "target/debug/libqt_ffi.so" ]; then echo "target/debug/libqt_ffi.so";
    else echo "target/debug/libqt_ffi.dylib"; fi
}
bindgen() { # $1=features $2=language $3=outdir
    cargo run -q -p qt_ffi --features "$1" --bin uniffi-bindgen -- \
        generate --library "$(lib_path)" --language "$2" --out-dir "$3"
}

# ---------- 1) Perfil de producao: Kotlin + Swift ----------
echo ">>> [producao] a compilar qt_ffi --features pq ..."
cargo build -p qt_ffi --features pq
rm -rf ffi/bindings/kotlin ffi/bindings/swift
mkdir -p ffi/bindings
echo ">>> A gerar bindings Kotlin (Android)..."
bindgen pq kotlin ffi/bindings/kotlin
echo ">>> A gerar bindings Swift (iOS)..."
bindgen pq swift ffi/bindings/swift

KT=ffi/bindings/kotlin/uniffi/qt_ffi/qt_ffi.kt
grep -q "class QtClient" "$KT" \
    || { echo "ERRO: QtClient nao encontrado nos bindings Kotlin (feature pq?)"; exit 1; }
# Nenhuma funcao que exponha chaves privadas / estado de ratchet pode existir no Kotlin.
for sym in generateDhKeypair generateSigningKeypair generateNoiseStaticKeypair x3dhInitiate x3dhRespond \
           ratchetInitAsInitiator ratchetInitAsResponder ratchetEncrypt ratchetDecrypt unsealSender sealSender \
           newInitiator newResponder; do
    if grep -q "$sym" "$KT"; then
        echo "ERRO: '$sym' (API legacy) esta nos bindings Kotlin de producao"; exit 1;
    fi
done
echo ">>> OK: bindings Kotlin/Swift so expoem a API de alto nivel (QtClient)."

# ---------- 2) Build de teste: Python (inclui API legacy) ----------
echo ">>> [teste] a compilar qt_ffi --features pq,legacy-api ..."
cargo build -p qt_ffi --features pq,legacy-api
echo ">>> A gerar bindings Python (so para testes locais)..."
bindgen pq,legacy-api python ffi/bindings
cp "$(lib_path)" ffi/bindings/

echo ">>> A correr os testes Python contra os bindings gerados..."
python3 ffi/tests/test_ffi_bindings.py
python3 ffi/tests/test_qt_client.py

echo ">>> Concluido. Kotlin/Swift: ffi/bindings/{kotlin,swift}/   Python (testes): ffi/bindings/qt_ffi.py"
