"""Página Denúncias: lista as denúncias (abertas com a chave do operador), apaga-as e bane contas."""
from __future__ import annotations

import time

from PySide6.QtCore import QProcess, QProcessEnvironment, Qt, Signal
from PySide6.QtGui import QColor, QTextCharFormat
from PySide6.QtWidgets import QHBoxLayout, QMessageBox, QPlainTextEdit, QPushButton, QVBoxLayout, QWidget

from .page_relay import Page
from .reports import REASONS, build_command, parse_bans, parse_reports
from .theme import C
from .widgets import Card, label


def _when(ms: int | None) -> str:
    return time.strftime("%d/%m/%Y %H:%M", time.localtime(ms / 1000)) if ms else "?"


class ReportsPage(Page):
    finished = Signal(bool, str)  # sucesso, descrição (para notificações)

    def __init__(self, get_cfg):
        super().__init__("Denúncias", "Abertas só neste PC, com a chave do operador")
        self.get_cfg = get_cfg
        self.proc: QProcess | None = None
        self.pending: tuple[str, dict] | None = None

        self.refresh_btn = QPushButton("↻  Atualizar")
        self.refresh_btn.setObjectName("primary")
        self.refresh_btn.clicked.connect(self.refresh)
        self.actions.addWidget(self.refresh_btn)

        self.status = label("Carrega em Atualizar para ver as denúncias.", "pageSub")
        self.root.addWidget(self.status)

        self.section("Denúncias")
        self.list_box = QVBoxLayout()
        self.list_box.setSpacing(10)
        holder = QWidget()
        holder.setLayout(self.list_box)
        self.root.addWidget(holder)

        self.section("Contas banidas")
        self.bans_box = QVBoxLayout()
        self.bans_box.setSpacing(6)
        bans = QWidget()
        bans.setLayout(self.bans_box)
        self.root.addWidget(bans)

        self.section("Registo")
        self.log = QPlainTextEdit()
        self.log.setReadOnly(True)
        self.log.setMaximumBlockCount(2000)
        self.log.setMinimumHeight(120)
        self.root.addWidget(self.log, 1)

    # --- registo ----------------------------------------------------------
    def _write(self, text: str, color: str = C["text"]):
        fmt = QTextCharFormat()
        fmt.setForeground(QColor(color))
        cur = self.log.textCursor()
        cur.movePosition(cur.MoveOperation.End)
        cur.insertText(text if text.endswith("\n") else text + "\n", fmt)
        self.log.setTextCursor(cur)
        self.log.ensureCursorVisible()

    @staticmethod
    def _clear(box: QVBoxLayout):
        while box.count():
            w = box.takeAt(0).widget()
            if w:
                w.deleteLater()

    # --- ações ------------------------------------------------------------
    def refresh(self):
        self._run("listar", {})

    def _ask_delete(self, report_id: int):
        ok = QMessageBox.question(self, "Apagar denúncia", f"Apagar a denúncia {report_id} do relay?\n\nNão se recupera.",
                                  QMessageBox.Yes | QMessageBox.Cancel, QMessageBox.Cancel)
        if ok == QMessageBox.Yes:
            self._run("apagar", {"report_id": report_id})

    def _ask_ban(self, user: str):
        ok = QMessageBox.question(self, "Banir conta",
                                  f"Banir a conta\n{user}?\n\nO relay apaga o que guarda dela e deixa de a aceitar. "
                                  "A pessoa pode criar um ID novo.",
                                  QMessageBox.Yes | QMessageBox.Cancel, QMessageBox.Cancel)
        if ok == QMessageBox.Yes:
            self._run("banir", {"user": user})

    def _ask_unban(self, user: str):
        ok = QMessageBox.question(self, "Desbanir conta", f"Voltar a aceitar a conta\n{user}?",
                                  QMessageBox.Yes | QMessageBox.Cancel, QMessageBox.Cancel)
        if ok == QMessageBox.Yes:
            self._run("desbanir", {"user": user})

    def _run(self, action: str, kw: dict):
        if self.proc is not None:
            return
        try:
            cmd = build_command(self.get_cfg(), action, **kw)
        except ValueError as exc:
            QMessageBox.warning(self, "Não é possível continuar", str(exc))
            return
        self.pending = (action, kw)
        self._write(f"\n[{time.strftime('%H:%M:%S')}] $ {cmd.display()}", C["faint"])
        proc = QProcess(self)
        env = QProcessEnvironment.systemEnvironment()
        env.insert("PATH", cmd.path_env)
        env.insert("CARGO_TERM_COLOR", "never")
        proc.setProcessEnvironment(env)
        proc.setWorkingDirectory(cmd.cwd)
        proc.finished.connect(self._on_finished)
        proc.errorOccurred.connect(self._on_error)
        self.proc = proc
        self._busy(True, "A falar com o relay… (a primeira vez compila a ferramenta)")
        proc.start(cmd.program, cmd.args)

    def _on_error(self, err):
        if err == QProcess.FailedToStart and self.proc:
            self._finish(False, "Não foi possível arrancar o cargo.", "", "")

    def _on_finished(self, code: int, status):
        if not self.proc:
            return
        out = bytes(self.proc.readAllStandardOutput()).decode("utf-8", "replace")
        err = bytes(self.proc.readAllStandardError()).decode("utf-8", "replace")
        ok = status == QProcess.NormalExit and code == 0
        self._finish(ok, "" if ok else f"Falhou (código {code}). Vê o registo.", out, err)

    def _finish(self, ok: bool, msg: str, out: str, err: str):
        action, kw = self.pending or ("", {})
        self.proc.deleteLater()
        self.proc = None
        self.pending = None
        for line in err.splitlines():
            if line.strip():
                self._write("  " + line, C["red"] if "ERRO" in line else C["muted"])
        self._busy(False, "")
        if not ok:
            self.status.setText("✗ " + msg)
            self.status.setStyleSheet(f"color: {C['red']};")
            self.finished.emit(False, msg)
            return
        try:
            if action == "listar":
                self._show_reports(parse_reports(out))
                self._run_after("banidos", {})
            elif action == "banidos":
                self._show_bans(parse_bans(out))
            else:
                done = {"apagar": "Denúncia apagada.", "banir": "Conta banida.", "desbanir": "Conta desbanida."}[action]
                self._write("✓ " + done, C["green"])
                self.finished.emit(True, done)
                self._run_after("listar", {})
        except ValueError as exc:
            self.status.setText(f"✗ {exc}")

    def _run_after(self, action: str, kw: dict):
        # Corre depois de o processo anterior ter saído do caminho.
        from PySide6.QtCore import QTimer
        QTimer.singleShot(0, lambda: self._run(action, kw))

    def _busy(self, busy: bool, text: str):
        self.refresh_btn.setEnabled(not busy)
        if busy:
            self.status.setStyleSheet("")
            self.status.setText(text)

    # --- desenho ----------------------------------------------------------
    def _show_reports(self, reports: list[dict]):
        self._clear(self.list_box)
        self.status.setStyleSheet("")
        self.status.setText(f"{len(reports)} denúncia(s) no relay. Apagam-se sozinhas ao fim de 30 dias.")
        if not reports:
            self.list_box.addWidget(label("Sem denúncias.", "cardNote"))
        for r in reports:
            self.list_box.addWidget(self._report_card(r))

    def _report_card(self, r: dict) -> Card:
        card = Card(padding=14)
        d = r.get("denuncia") or {}
        card.lay.addWidget(label(f"DENÚNCIA {r['id']}  ·  recebida {_when(r.get('recebida_em_ms'))}", "cardTitle"))
        if "erro" in r:
            card.lay.addWidget(label(f"Não abre: {r['erro']}", "cardNote"))
        else:
            relay_id = str(d.get("denunciado_relay_id", ""))
            card.lay.addWidget(label(f"Motivo: {REASONS.get(d.get('motivo'), d.get('motivo', '?'))}", wrap=True))
            card.lay.addWidget(label(f"Denunciado (id do relay): {relay_id}", "cardNote", wrap=True))
            msgs = d.get("mensagens") or []
            if not msgs:
                card.lay.addWidget(label("Sem mensagens incluídas.", "cardNote"))
            for m in msgs:
                who = "Denunciado" if m.get("de_denunciado") else "Quem denunciou"
                extra = "  [anexo não incluído]" if m.get("anexo") else ""
                text = label(f"{_when(m.get('enviada_em_ms'))}  {who}: {m.get('texto', '')}{extra}", wrap=True)
                text.setTextInteractionFlags(Qt.TextSelectableByMouse)
                card.lay.addWidget(text)
        row = QHBoxLayout()
        row.addStretch(1)
        if "erro" not in r and d.get("denunciado_relay_id"):
            ban = QPushButton("⛔  Banir")
            ban.setObjectName("danger")
            ban.clicked.connect(lambda _=False, u=str(d["denunciado_relay_id"]): self._ask_ban(u))
            row.addWidget(ban)
        rm = QPushButton("Apagar denúncia")
        rm.clicked.connect(lambda _=False, i=r["id"]: self._ask_delete(i))
        row.addWidget(rm)
        card.lay.addLayout(row)
        return card

    def _show_bans(self, bans: list[str]):
        self._clear(self.bans_box)
        if not bans:
            self.bans_box.addWidget(label("Nenhuma conta banida.", "cardNote"))
        for u in bans:
            row = QWidget()
            lay = QHBoxLayout(row)
            lay.setContentsMargins(0, 0, 0, 0)
            t = label(u, "cardNote")
            t.setTextInteractionFlags(Qt.TextSelectableByMouse)
            lay.addWidget(t, 1)
            b = QPushButton("Desbanir")
            b.clicked.connect(lambda _=False, x=u: self._ask_unban(x))
            lay.addWidget(b)
            self.bans_box.addWidget(row)
