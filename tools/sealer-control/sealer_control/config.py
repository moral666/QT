"""Configuração persistente (~/.config/sealer-control/config.json, modo 600).

Não guarda credenciais AWS: usa o perfil do AWS CLI.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any

CONFIG_DIR = Path.home() / ".config" / "sealer-control"
CONFIG_PATH = CONFIG_DIR / "config.json"
CACHE_PATH = CONFIG_DIR / "cache.json"
LEGACY_PATH = Path.home() / ".config" / "sealer-aws-monitor" / "config.json"

DEFAULTS: dict[str, Any] = {
    # AWS
    "region": "eu-north-1",
    "profile": "default",
    "instance_id": "",
    "namespace": "Sealer/Relay",
    "refresh_seconds": 60,
    # Site e relay
    "site_url": "https://sealer.pt/",
    "estado_url": "https://relay.sealer.pt/estado.json",
    "site_interval": 60,
    "slow_threshold": 1.0,
    # Badges
    "repo_path": "~/QT-repo",
    "badge_key": "~/Documentos/SENSIVEL /badge-emissor.key",
    "badge_db": "~/Documentos/SENSIVEL /badge-emissor.db",
    # Geral
    "notifications": True,
    "minimize_to_tray": True,
}


def _read(path: Path) -> dict[str, Any]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        return data if isinstance(data, dict) else {}
    except (OSError, json.JSONDecodeError):
        return {}


def _write_private(path: Path, data: dict[str, Any]) -> None:
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(data, indent=2, default=str), encoding="utf-8")
    os.chmod(tmp, 0o600)
    tmp.replace(path)


def load_config() -> dict[str, Any]:
    cfg = DEFAULTS.copy()
    saved = _read(CONFIG_PATH)
    if not saved:
        # Aproveita as definições do antigo sealer_aws_monitor
        saved = {k: v for k, v in _read(LEGACY_PATH).items() if k != "refresh_seconds"}
    cfg.update({k: v for k, v in saved.items() if k in cfg})
    return cfg


def save_config(cfg: dict[str, Any]) -> None:
    _write_private(CONFIG_PATH, {k: cfg[k] for k in DEFAULTS if k in cfg})


def load_cache() -> dict[str, Any]:
    return _read(CACHE_PATH)


def save_cache(cache: dict[str, Any]) -> None:
    try:
        _write_private(CACHE_PATH, cache)
    except OSError:
        pass


def expand(path: str) -> str:
    return os.path.expandvars(os.path.expanduser(path))
