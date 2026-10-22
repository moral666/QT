"""Denúncias e banimentos através de `cargo run --example denuncias`.

Como nos badges, o comando é uma lista de argumentos e corre sem shell. A chave do
operador fica num ficheiro (pen ou pasta SENSIVEL) e nunca passa pela app: é a
ferramenta em Rust que a lê, abre as denúncias e fala com o relay.
"""
from __future__ import annotations

import json
import os
import re
import shutil

from .badges import BadgeCommand
from .config import expand

RELAY_ID_RE = re.compile(r"^[0-9a-f]{64}$")
ACTIONS = ("listar", "banidos", "apagar", "banir", "desbanir")


def validate_relay_id(user: str) -> str | None:
    """None se for um id do relay válido (64 caracteres hex), senão a explicação."""
    if not RELAY_ID_RE.match(user.strip().lower()):
        return "O id do relay tem 64 caracteres hexadecimais (como aparece na denúncia)."
    return None


def build_command(cfg: dict, action: str, *, report_id: int | None = None, user: str | None = None) -> BadgeCommand:
    if action not in ACTIONS:
        raise ValueError(f"Ação desconhecida: {action}")
    repo, key = expand(cfg["repo_path"]), expand(cfg["operator_key"])
    if not os.path.isdir(repo):
        raise ValueError(f"Não encontrei o repositório em {repo}")
    if not os.path.isfile(key):
        raise ValueError("Não encontrei a chave do operador. Confirma o caminho nas Definições (a pen está ligada?).")
    path_env = os.pathsep.join([expand("~/.cargo/bin"), "/usr/local/bin", "/usr/bin", "/bin", os.environ.get("PATH", "")])
    cargo = shutil.which("cargo", path=path_env)
    if not cargo:
        raise ValueError("Não encontrei o cargo (Rust). Instala com rustup.")
    args = ["run", "-q", "--release", "-p", "qt_server", "--example", "denuncias", "--", action, "--key", key]
    if action in ("listar", "banidos"):
        args.append("--json")
    if action == "apagar":
        if report_id is None or report_id < 1:
            raise ValueError("Falta o número da denúncia.")
        args += ["--id", str(report_id)]
    if action in ("banir", "desbanir"):
        problem = validate_relay_id(user or "")
        if problem:
            raise ValueError(problem)
        args += ["--user", (user or "").strip().lower()]
    return BadgeCommand(cargo, args, repo, path_env)


def parse_reports(stdout: str) -> list[dict]:
    """Lista de `listar --json`: [{id, recebida_em_ms, denuncia | erro}]."""
    data = json.loads(stdout.strip() or "[]")
    if not isinstance(data, list):
        raise ValueError("resposta inesperada da ferramenta")
    return [d for d in data if isinstance(d, dict) and isinstance(d.get("id"), int)]


def parse_bans(stdout: str) -> list[str]:
    data = json.loads(stdout.strip() or "[]")
    if not isinstance(data, list):
        raise ValueError("resposta inesperada da ferramenta")
    return [u for u in data if isinstance(u, str)]


REASONS = {"spam": "Spam ou burla", "assedio": "Assédio ou ameaças", "ilegal": "Conteúdo ilegal", "outro": "Outro"}
