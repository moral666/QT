"""Verificação do site e leitura do estado.json do relay (sem dependências externas)."""
from __future__ import annotations

import json
import time
import urllib.error
import urllib.request
from typing import Any

UA = {"User-Agent": "SealerControl/1.0 (+monitor local)", "Cache-Control": "no-cache"}
STALE_S = 3600


def check_site(url: str, slow: float, timeout: float = 5.0) -> dict[str, Any]:
    """Pedido GET ao site. Devolve tone ok/warn/bad, código HTTP e tempo em ms."""
    start = time.perf_counter()
    code: int | None = None
    error = ""
    try:
        req = urllib.request.Request(url, headers=UA)
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            resp.read(2048)
            code = resp.status
    except urllib.error.HTTPError as exc:
        code = exc.code
    except (urllib.error.URLError, TimeoutError, OSError) as exc:
        error = str(getattr(exc, "reason", exc))
    ms = (time.perf_counter() - start) * 1000
    if code is None:
        return {"tone": "bad", "title": "Site em baixo", "detail": error or "Sem resposta", "code": None, "ms": None, "ts": time.time()}
    if code != 200:
        return {"tone": "bad", "title": f"Erro no servidor ({code})", "detail": f"HTTP {code}", "code": code, "ms": ms, "ts": time.time()}
    if ms > slow * 1000:
        return {"tone": "warn", "title": "Online, mas lento", "detail": f"HTTP 200 · {ms:.0f} ms", "code": code, "ms": ms, "ts": time.time()}
    return {"tone": "ok", "title": "Online", "detail": f"HTTP 200 · {ms:.0f} ms", "code": code, "ms": ms, "ts": time.time()}


def fetch_estado(url: str, timeout: float = 5.0) -> dict[str, Any]:
    req = urllib.request.Request(url, headers=UA)
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read(65536).decode("utf-8"))


def age_text(seconds: float | None) -> str:
    if seconds is None:
        return "—"
    s = int(max(0, seconds))
    if s < 90:
        return f"há {s} s"
    if s < 3600:
        return f"há {s // 60} min"
    h, m = divmod(s // 60, 60)
    return f"há {h} h {m:02d} min" if h < 24 else f"há {h // 24} dia(s)"


def interpret_estado(js: dict[str, Any], now: float | None = None) -> dict[str, Any]:
    """Traduz o estado.json do relay, marcando valores antigos.

    O site soma a mediana do envio com a última mediana de entrega, mesmo que essa
    entrega tenha horas. Aqui só se soma quando a entrega é da última hora.
    """
    now = now or time.time()
    out: dict[str, Any] = {"raw": js}
    out["updated_age"] = now - js["atualizado"] if js.get("atualizado") else None
    out["relay_stale"] = out["updated_age"] is None or out["updated_age"] > 300

    out["ida"] = js.get("ida_mediana_ms")
    out["ida_p95"] = js.get("ida_p95_ms")
    out["ida_n"] = js.get("ida_amostras") or 0
    out["ida_age"] = now - js["ultima_ida_medicao"] if js.get("ultima_ida_medicao") else None

    if js.get("entrega_mediana_ms") is not None:
        out["entrega"], out["entrega_age"], out["entrega_live"] = js["entrega_mediana_ms"], out["updated_age"], True
    else:
        out["entrega"] = js.get("ultima_mediana_ms")
        out["entrega_age"] = now - js["ultima_medicao"] if js.get("ultima_medicao") else None
        out["entrega_live"] = False
    out["entrega_p95"] = js.get("entrega_p95_ms")
    out["mensagens"] = js.get("mensagens") or 0
    out["entrega_stale"] = out["entrega"] is None or out["entrega_age"] is None or out["entrega_age"] > STALE_S

    out["total"] = None if out["ida"] is None or out["entrega_stale"] else out["ida"] + out["entrega"]
    out["site_total"] = (out["ida"] or 0) + (out["entrega"] or 0) if out["entrega"] is not None else None

    if out["relay_stale"]:
        out["tone"] = "bad"
    elif out["ida"] is not None and out["ida"] > 300:
        out["tone"] = "warn"
    else:
        out["tone"] = "ok"
    return out
