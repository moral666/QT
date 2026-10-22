"""Emissão de badges através de `cargo run --example badge -- dar`.

O comando é construído como lista de argumentos e corre sem shell,
por isso o ID escrito na caixa nunca é interpretado pelo bash.
"""
from __future__ import annotations

import os
import re
import shutil
from dataclasses import dataclass

from .config import expand

VALID_BADGES = ("founder", "cofounder", "dev", "tester", "contributor", "nenhum")
USER_RE = re.compile(r"^qt1:[A-Za-z0-9._~+/=-]{8,200}$")


def validate_user(user: str) -> str | None:
    """Devolve None se o ID for válido, ou a explicação do problema."""
    user = user.strip()
    if not user or user == "qt1:":
        return "Escreve o identificador da conta (qt1:…)."
    if not user.startswith("qt1:"):
        return "O identificador tem de começar por qt1:"
    if any(ch.isspace() for ch in user):
        return "O identificador não pode ter espaços."
    if not USER_RE.match(user):
        return "Formato inválido: depois de qt1: só letras, números e . _ ~ + / = -"
    return None


@dataclass
class BadgeCommand:
    program: str
    args: list[str]
    cwd: str
    path_env: str

    def display(self) -> str:
        # Mostra o comando sem expor caminhos completos da pasta sensível
        shown = []
        hide = False
        for a in self.args:
            shown.append("…/" + os.path.basename(a) if hide else a)
            hide = a in ("--key", "--db")
        return "cargo " + " ".join(shown)


def build_command(cfg: dict, user: str, badge: str) -> BadgeCommand:
    if badge not in VALID_BADGES:
        raise ValueError(f"Badge desconhecido: {badge}")
    problem = validate_user(user)
    if problem:
        raise ValueError(problem)
    repo, key, db = expand(cfg["repo_path"]), expand(cfg["badge_key"]), expand(cfg["badge_db"])
    if not os.path.isdir(repo):
        raise ValueError(f"Não encontrei o repositório em {repo}")
    if not os.path.isfile(key):
        raise ValueError("Não encontrei a chave do emissor. Confirma o caminho nas Definições (a pen está ligada?).")
    path_env = os.pathsep.join([expand("~/.cargo/bin"), "/usr/local/bin", "/usr/bin", "/bin", os.environ.get("PATH", "")])
    cargo = shutil.which("cargo", path=path_env)
    if not cargo:
        raise ValueError("Não encontrei o cargo (Rust). Instala com rustup.")
    args = ["run", "--release", "-p", "qt_server", "--example", "badge", "--",
            "dar", "--key", key, "--db", db, "--user", user.strip(), "--badge", badge]
    return BadgeCommand(cargo, args, repo, path_env)
