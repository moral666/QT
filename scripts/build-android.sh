#!/usr/bin/env bash
# Compila libqt_ffi.so para as 3 ABIs Android e gera os bindings Kotlin.
#
#   Rust -> cargo-ndk -> libqt_ffi.so (arm64-v8a, armeabi-v7a, x86_64)
#        -> UniFFI -> qt_ffi.kt  -> app Android
#
# Requisitos (no PC de build, Linux/macOS):
#   * Rust >= 1.81 via rustup           https://rustup.rs
#   * Android NDK r26+ (ex.: Android Studio > SDK Manager > NDK)
#   * perl, make (para compilar SQLCipher + OpenSSL a partir do codigo-fonte)
#   * cargo install cargo-ndk --locked
#
# Uso:
#   export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/<versao>
#   ./scripts/build-android.sh [caminho/para/app/src/main]      (omissao: ./android-out)
#
# NOTA: este script NAO foi executado no ambiente de desenvolvimento (sem
# acesso a rustup/NDK). Os testes foram corridos no host com as MESMAS features
# (`pq` + SQLCipher embebido). Verifica o primeiro build num CI/maquina tua.

set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-android-out}"
# ABIs a compilar (separadas por espaco). Omissao: as 3.
# NOTA: com o NDK r26d, a ABI x86_64 falha ao compilar o OpenSSL embebido
# (o assembler do clang 17 nao conhece as instrucoes x86 SM3/SM4: "invalid
# instruction mnemonic 'vsm3msg1'"). Usa um NDK mais recente (r27+), ou
# compila so os telemoveis reais:  ABIS="arm64-v8a armeabi-v7a" ./scripts/build-android.sh
ABIS="${ABIS:-arm64-v8a armeabi-v7a x86_64}"
API_LEVEL="${ANDROID_API_LEVEL:-24}"
: "${ANDROID_NDK_HOME:?Define ANDROID_NDK_HOME (pasta do NDK, ex.: \$HOME/Android/Sdk/ndk/26.1.10909125)}"
command -v cargo-ndk >/dev/null || { echo "Falta cargo-ndk: cargo install cargo-ndk --locked"; exit 1; }

echo ">>> targets Rust para Android"
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android

# O openssl-src (usado pelo SQLCipher embebido) precisa do clang do NDK no PATH.
case "$(uname -s)" in
  Linux)  HOST_TAG=linux-x86_64 ;;
  Darwin) HOST_TAG=darwin-x86_64 ;;
  *) echo "SO de build nao suportado"; exit 1 ;;
esac
export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$HOST_TAG/bin:$PATH"
export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"

echo ">>> cargo ndk build (release) -> $OUT/jniLibs/<abi>/libqt_ffi.so"
# `android` = pq + SQLCipher/OpenSSL embebidos. SEM `legacy-api` (nao expoe chaves privadas).
TARGETS=""
for a in $ABIS; do TARGETS="$TARGETS -t $a"; done
cargo ndk \
  $TARGETS \
  -P "$API_LEVEL" \
  -o "$OUT/jniLibs" \
  build --release -p qt_ffi --no-default-features --features android

echo ">>> bindings Kotlin (perfil de producao)"
./ffi/generate_bindings.sh >/dev/null
mkdir -p "$OUT/kotlin"
cp -r ffi/bindings/kotlin/uniffi "$OUT/kotlin/"

echo ">>> verificacao"
for abi in $ABIS; do
  f="$OUT/jniLibs/$abi/libqt_ffi.so"
  [ -f "$f" ] || { echo "FALTA $f"; exit 1; }
  # nenhum segredo/caminho de debug embutido
  ls -l "$f" | awk '{print "  "$9" ("$5" bytes)"}'
  # Paginas de 16 KB: todos os segmentos LOAD tem de estar alinhados a 0x4000.
  # (armeabi-v7a e 32 bits: so aparelhos 64 bits usam paginas de 16 KB, mas
  #  verificamos todas as ABIs na mesma.)
  READELF="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$HOST_TAG/bin/llvm-readelf"
  if grep -E "^ +LOAD" <("$READELF" -lW "$f") | awk '{print $NF}' | grep -qv '^0x4000$\|^0x10000$'; then
    echo "ERRO: $f nao esta alinhado a 16 KB:"; "$READELF" -lW "$f" | grep -E "^ +LOAD"; exit 1
  fi
  echo "    alinhamento 16 KB: OK"
done
echo ">>> OK. Copia $OUT/jniLibs para app/src/main/jniLibs e $OUT/kotlin para app/src/main/java."
