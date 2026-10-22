"""Corre funções bloqueantes (rede, AWS) fora da thread da interface."""
from __future__ import annotations

from typing import Any, Callable

from PySide6.QtCore import QObject, QRunnable, Qt, QThreadPool, Signal

_alive: set["_Bridge"] = set()


class _Bridge(QObject):
    ok = Signal(object)
    err = Signal(str)


class _Job(QRunnable):
    def __init__(self, fn: Callable[[], Any], bridge: _Bridge):
        super().__init__()
        self.fn, self.bridge = fn, bridge

    def run(self):
        try:
            result = self.fn()
        except Exception as exc:  # o erro é mostrado na interface
            self.bridge.err.emit(f"{type(exc).__name__}: {exc}" if not str(exc) else str(exc))
        else:
            self.bridge.ok.emit(result)


def run_async(fn: Callable[[], Any], on_ok: Callable[[Any], None], on_err: Callable[[str], None]) -> None:
    """Executa fn num thread do pool; os callbacks correm na thread da interface."""
    bridge = _Bridge()  # criado na thread da interface
    _alive.add(bridge)

    def done_ok(result):
        _alive.discard(bridge)
        on_ok(result)

    def done_err(msg):
        _alive.discard(bridge)
        on_err(msg)

    bridge.ok.connect(done_ok, Qt.QueuedConnection)
    bridge.err.connect(done_err, Qt.QueuedConnection)
    QThreadPool.globalInstance().start(_Job(fn, bridge))
