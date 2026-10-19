#!/usr/bin/env bash
# SEALER para iPhone: compila tudo e abre o projeto no Xcode. Correr num Mac:
#   bash compilar-iphone.sh
set -euo pipefail
cd "$(dirname "$0")"

command -v xcodebuild >/dev/null || { echo "Falta o Xcode (instala pela App Store e abre-o uma vez)."; exit 1; }
if ! command -v cargo >/dev/null; then
  [ -x "$HOME/.cargo/bin/cargo" ] && export PATH="$HOME/.cargo/bin:$PATH" || {
    echo ">>> a instalar o Rust"; curl -sSf https://sh.rustup.rs | sh -s -- -y; export PATH="$HOME/.cargo/bin:$PATH"; }
fi
echo ">>> Rust atualizado"; rustup update stable
if ! command -v xcodegen >/dev/null; then
  command -v brew >/dev/null || { echo "Falta o Homebrew (https://brew.sh) para instalar o xcodegen."; exit 1; }
  echo ">>> a instalar o xcodegen"; brew install xcodegen
fi

./scripts/build-ios.sh
cd ios && xcodegen && open SealerMessenger.xcodeproj
echo
echo "Pronto. No Xcode: SealerMessenger > Signing & Capabilities > Team; liga o iPhone e carrega em Run."
