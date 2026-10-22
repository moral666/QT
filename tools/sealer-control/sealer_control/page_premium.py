"""Página Premium: ligar o bloqueio para todos, dar ou tirar premium a uma conta e ver quem tem.

O relay é quem decide (0.9.21): com o bloqueio ligado, recusa anexos e chamadas a contas sem
premium, e as apps fecham as funções pagas. Desligado, está tudo aberto para toda a gente.
"""
from __future__ import annotations

import time

from PySide6.QtCore import QProcess, QProcessEnvironment, Qt, QTimer, Signal
from PySide6.QtGui import QColor, QTextCharFormat
from PySide6.QtWidgets import (QComboBox, QHBoxLayout, QLineEdit, QMessageBox, QPlainTextEdit, QPushButton,
                               QVBoxLayout, QWidget)

from .page_relay import Page
from .reports import build_command, parse_enforce, parse_premium, validate_public_id
from .theme import C
from .widgets import Card, label

DURATIONS = (("1 mês", 30), ("3 meses", 90), ("1 ano", 365), ("Sem fim", None))
FOREVER_MS = 253_402_300_799_000


def _until(ms: int) -> str:
    return "sem fim" if ms >= FOREVER_MS else time.strftime("%d/%m/%Y", time.localtime(ms / 1000))


class PremiumPage(Page):
    finished = Signal(bool, str)

    def __init__(self, get_cfg):
        super().__init__("Premium", "Quem tem premium e o bloqueio para todos (a chave do operador assina cada pedido)")
        self.get_cfg = get_cfg
        self.proc: QProcess | None = None
        self.pending: str = ""
        self.enforced: bool | None = None

        self.refresh_btn = QPushButton("↻  Atualizar")
        self.refresh_btn.setObjectName("primary")
        self.refresh_btn.clicked.connect(self.refresh)
        self.actions.addWidget(self.refresh_btn)

        lock = Card(padding=18)
        lock.lay.addWidget(label("BLOQUEIO PREMIUM (PARA TODOS)", "cardTitle"))
        self.lock_state = label("Estado desconhecido: carrega em Atualizar.", "cardNote")
        lock.lay.addWidget(self.lock_state)
        lrow = QHBoxLayout()
        self.lock_btn = QPushButton("Ligar o bloqueio")
        self.lock_btn.setEnabled(False)
        self.lock_btn.clicked.connect(self._toggle_lock)
        lrow.addWidget(self.lock_btn)
        lrow.addStretch(1)
        lock.lay.addLayout(lrow)
        self.root.addWidget(lock)

        card = Card(padding=18)
        card.lay.addWidget(label("ID DA CONTA", "cardTitle"))
        self.user = QLineEdit("qt1:")
        self.user.setClearButtonEnabled(True)
        self.user.textChanged.connect(self._validate)
        card.lay.addWidget(self.user)
        self.hint = label("", "cardNote")
        card.lay.addWidget(self.hint)
        row = QHBoxLayout()
        self.duration = QComboBox()
        for text, _ in DURATIONS:
            self.duration.addItem(text)
        self.duration.setCurrentIndex(2)
        row.addWidget(self.duration)
        self.give_btn = QPushButton("★  Dar premium")
        self.give_btn.setObjectName("primary")
        self.give_btn.clicked.connect(self._give)
        row.addWidget(self.give_btn)
        self.take_btn = QPushButton("Tirar premium")
        self.take_btn.setObjectName("danger")
        self.take_btn.clicked.connect(self._take)
        row.addWidget(self.take_btn)
        row.addStretch(1)
        card.lay.addLayout(row)
        self.root.addWidget(card)

        self.status = label("", "pageSub")
        self.root.addWidget(self.status)
        self.section("Contas com premium")
        self.list_box = QVBoxLayout()
        holder = QWidget()
        holder.setLayout(self.list_box)
        self.root.addWidget(holder)
        self.section("Registo")
        self.log = QPlainTextEdit()
        self.log.setReadOnly(True)
        self.log.setMinimumHeight(110)
        self.root.addWidget(self.log, 1)
        self._validate()

    def _write(self, text: str, color: str = C["text"]):
        fmt = QTextCharFormat()
        fmt.setForeground(QColor(color))
        cur = self.log.textCursor()
        cur.movePosition(cur.MoveOperation.End)
        cur.insertText(text if text.endswith("\n") else text + "\n", fmt)
        self.log.setTextCursor(cur)

    def _validate(self):
        problem = validate_public_id(self.user.text())
        self.hint.setText("ID válido." if problem is None else problem)
        self.hint.setStyleSheet(f"color: {C['green'] if problem is None else C['muted']};")
        ok = problem is None and self.proc is None
        self.give_btn.setEnabled(ok)
        self.take_btn.setEnabled(ok)

    def refresh(self):
        """Estado do bloqueio e depois a lista (um pedido de cada vez)."""
        self._run("premium-bloqueio", {})

    def _show_lock(self, on: bool):
        self.enforced = on
        if on:
            self.lock_state.setText("LIGADO: quem não tem premium só envia texto (anexos e chamadas recusados pelo relay).")
            self.lock_state.setStyleSheet(f"color: {C['red']};")
            self.lock_btn.setText("Desligar o bloqueio")
            self.lock_btn.setObjectName("")
        else:
            self.lock_state.setText("Desligado: todas as funções estão abertas a toda a gente.")
            self.lock_state.setStyleSheet(f"color: {C['green']};")
            self.lock_btn.setText("Ligar o bloqueio")
            self.lock_btn.setObjectName("danger")
        self.lock_btn.style().unpolish(self.lock_btn)
        self.lock_btn.style().polish(self.lock_btn)
        self.lock_btn.setEnabled(self.proc is None)

    def _toggle_lock(self):
        if self.enforced is None:
            return
        on = not self.enforced
        text = ("Ligar o bloqueio?\n\nQuem não tiver premium deixa logo de enviar fotos, voz, ficheiros, "
                "de fazer chamadas e de criar grupos.") if on else "Desligar o bloqueio?\n\nTudo fica aberto para toda a gente."
        if QMessageBox.question(self, "Bloqueio premium", text,
                                QMessageBox.Yes | QMessageBox.Cancel, QMessageBox.Cancel) == QMessageBox.Yes:
            self._run("premium-bloqueio", {"on": on})

    def _take_listed(self, user_id: str):
        if QMessageBox.question(self, "Tirar premium", f"Tirar o premium a\n{user_id}?",
                                QMessageBox.Yes | QMessageBox.Cancel, QMessageBox.Cancel) == QMessageBox.Yes:
            self._run("premium-tirar", {"user": user_id})

    def _give(self):
        text, days = DURATIONS[self.duration.currentIndex()]
        user = self.user.text().strip().lower()
        if QMessageBox.question(self, "Dar premium", f"Dar premium ({text}) a\n{user}?",
                                QMessageBox.Yes | QMessageBox.Cancel, QMessageBox.Cancel) == QMessageBox.Yes:
            self._run("premium-dar", {"user": user, "days": days})

    def _take(self):
        user = self.user.text().strip().lower()
        if QMessageBox.question(self, "Tirar premium", f"Tirar o premium a\n{user}?",
                                QMessageBox.Yes | QMessageBox.Cancel, QMessageBox.Cancel) == QMessageBox.Yes:
            self._run("premium-tirar", {"user": user})

    def _run(self, action: str, kw: dict):
        if self.proc is not None:
            return
        try:
            cmd = build_command(self.get_cfg(), action, **kw)
        except ValueError as exc:
            QMessageBox.warning(self, "Não é possível continuar", str(exc))
            return
        self.pending = action
        self._write(f"\n[{time.strftime('%H:%M:%S')}] $ {cmd.display()}", C["faint"])
        proc = QProcess(self)
        env = QProcessEnvironment.systemEnvironment()
        env.insert("PATH", cmd.path_env)
        env.insert("CARGO_TERM_COLOR", "never")
        proc.setProcessEnvironment(env)
        proc.setWorkingDirectory(cmd.cwd)
        proc.finished.connect(self._on_finished)
        self.proc = proc
        self.refresh_btn.setEnabled(False)
        self.lock_btn.setEnabled(False)
        self.status.setStyleSheet("")
        self.status.setText("A falar com o relay…")
        self._validate()
        proc.start(cmd.program, cmd.args)

    def _on_finished(self, code: int, status):
        if not self.proc:
            return
        out = bytes(self.proc.readAllStandardOutput()).decode("utf-8", "replace")
        err = bytes(self.proc.readAllStandardError()).decode("utf-8", "replace")
        self.proc.deleteLater()
        self.proc = None
        self.refresh_btn.setEnabled(True)
        self.lock_btn.setEnabled(self.enforced is not None)
        self._validate()
        for line in err.splitlines():
            if line.strip():
                self._write("  " + line, C["red"] if "ERRO" in line else C["muted"])
        if status != QProcess.NormalExit or code != 0:
            self.status.setText(f"✗ Falhou (código {code}). Vê o registo.")
            self.status.setStyleSheet(f"color: {C['red']};")
            self.finished.emit(False, "Falhou")
            return
        if self.pending == "premium-bloqueio":
            try:
                self._show_lock(parse_enforce(out))
            except ValueError as exc:
                self.status.setText(f"✗ {exc}")
                return
            QTimer.singleShot(0, lambda: self._run("premium-lista", {}))
            return
        if self.pending == "premium-lista":
            try:
                self._show(parse_premium(out))
            except ValueError as exc:
                self.status.setText(f"✗ {exc}")
            return
        done = out.strip() or "Feito."
        self._write("✓ " + done, C["green"])
        self.finished.emit(True, done)
        QTimer.singleShot(0, lambda: self._run("premium-lista", {}))

    def _show(self, entries: list[dict]):
        while self.list_box.count():
            w = self.list_box.takeAt(0).widget()
            if w:
                w.deleteLater()
        self.status.setStyleSheet("")
        self.status.setText(f"{len(entries)} conta(s) com premium.")
        if not entries:
            self.list_box.addWidget(label("Ninguém com premium.", "cardNote"))
        for e in entries:
            row = QWidget()
            lay = QHBoxLayout(row)
            lay.setContentsMargins(0, 0, 0, 0)
            t = label(f"{e['user_id'][:16]}…  até {_until(e['until_ms'])}", "cardNote")
            t.setToolTip(e["user_id"])
            t.setTextInteractionFlags(Qt.TextSelectableByMouse)
            lay.addWidget(t, 1)
            take = QPushButton("Tirar")
            take.setObjectName("danger")
            take.clicked.connect(lambda _=False, u=e["user_id"]: self._take_listed(u))
            lay.addWidget(take)
            self.list_box.addWidget(row)
