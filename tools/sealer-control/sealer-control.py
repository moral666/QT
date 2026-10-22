#!/usr/bin/env python3
"""Ponto de entrada (também usado pelo PyInstaller)."""
import sys

from sealer_control.app import main

if __name__ == "__main__":
    sys.exit(main())
