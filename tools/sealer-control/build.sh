#!/usr/bin/env bash
# Gera dist/sealer-control (um único executável).
set -euo pipefail
cd "$(dirname "$0")"
python3 -m venv .venv
. .venv/bin/activate
pip install -q -r requirements.txt pyinstaller
pyinstaller --noconfirm --onefile --windowed --name sealer-control \
  --exclude-module PySide6.QtWebEngineCore --exclude-module PySide6.QtQml --exclude-module PySide6.QtQuick \
  --exclude-module PySide6.QtPdf --exclude-module PySide6.QtMultimedia --exclude-module PySide6.Qt3DCore \
  --exclude-module tkinter \
  sealer-control.py
echo "Pronto: dist/sealer-control"
