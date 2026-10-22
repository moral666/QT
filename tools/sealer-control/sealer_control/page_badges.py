"""Página Badges: validação do ID, emissão sem shell e registo do resultado."""
from __future__ import annotations

import time

from PySide6.QtCore import QProcess, QProcessEnvironment, QRectF, QSize, Qt, Signal
from PySide6.QtGui import QColor, QPainter, QPen, QRadialGradient, QTextCharFormat
from PySide6.QtWidgets import (QAbstractButton, QGridLayout, QHBoxLayout, QLineEdit, QMessageBox,
                               QPlainTextEdit, QProgressBar, QPushButton)

from .badges import BadgeCommand, build_command, validate_user
from .page_relay import Page
from .theme import BADGES, C
from .widgets import Card, label


class BadgeButton(QAbstractButton):
    def __init__(self, key: str, name: str, dark: str, bright: str):
        super().__init__()
        self.key, self.name = key, name
        self.dark, self.bright = QColor(dark), QColor(bright)
        self.setCursor(Qt.PointingHandCursor)
        self.setMinimumSize(150, 120)
        self.setToolTip(f"Atribuir o badge {name}")

    def sizeHint(self) -> QSize:
        return QSize(170, 128)

    def paintEvent(self, _e):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        r = QRectF(self.rect()).adjusted(1, 1, -1, -1)
        hover = self.underMouse() and self.isEnabled()
        p.setPen(QPen(self.bright if hover else QColor(C["border"]), 1.5))
        p.setBrush(QColor(C["panel2"] if hover else C["panel"]))
        p.drawRoundedRect(r, 14, 14)
        if not self.isEnabled():
            p.setOpacity(0.4)

        cx, cy, rad = r.center().x(), r.top() + 48, 26
        grad = QRadialGradient(cx - 8, cy - 10, rad * 1.4)
        grad.setColorAt(0, self.bright)
        grad.setColorAt(1, self.dark)
        p.setPen(QPen(grad, 6))
        p.setBrush(Qt.NoBrush)
        p.drawEllipse(QRectF(cx - rad, cy - rad, rad * 2, rad * 2))
        glow = QColor(self.bright)
        glow.setAlpha(60 if hover else 25)
        p.setPen(Qt.NoPen)
        p.setBrush(glow)
        p.drawEllipse(QRectF(cx - rad + 8, cy - rad + 8, rad * 2 - 16, rad * 2 - 16))

        f = p.font()
        f.setBold(True)
        f.setPointSizeF(10)
        p.setFont(f)
        p.setPen(QColor(C["text"]))
        p.drawText(QRectF(r.left(), r.bottom() - 34, r.width(), 24), Qt.AlignCenter, self.name)

    def enterEvent(self, e):
        self.update()
        super().enterEvent(e)

    def leaveEvent(self, e):
        self.update()
        super().leaveEvent(e)


class BadgesPage(Page):
    finished = Signal(bool, str)  # sucesso, descrição (para notificações)

    def __init__(self, get_cfg):
        super().__init__("Badges", "Atribui ou retira medalhas a uma conta Sealer")
        self.get_cfg = get_cfg
        self.proc: QProcess | None = None
        self.current: tuple[str, str] | None = None
        self.started_at = 0.0

        card = Card(padding=18)
        card.lay.addWidget(label("IDENTIFICADOR DA CONTA", "cardTitle"))
        self.user = QLineEdit("qt1:")
        self.user.setPlaceholderText("qt1:…")
        self.user.setClearButtonEnabled(True)
        self.user.textChanged.connect(self._validate)
        card.lay.addWidget(self.user)
        self.hint = label("", "cardNote")
        card.lay.addWidget(self.hint)
        self.root.addWidget(card)

        self.section("Escolhe o badge")
        grid = QGridLayout()
        grid.setSpacing(10)
        self.buttons: list[BadgeButton] = []
        for i, (key, name, dark, bright) in enumerate(BADGES):
            b = BadgeButton(key, name, dark, bright)
            b.clicked.connect(lambda _=False, k=key, n=name: self.emit_badge(k, n))
            grid.addWidget(b, 0, i)
            self.buttons.append(b)
        self.root.addLayout(grid)

        row = QHBoxLayout()
        self.remove_btn = QPushButton("✕  Retirar badge")
        self.remove_btn.setObjectName("danger")
        self.remove_btn.clicked.connect(lambda: self.emit_badge("nenhum", "Nenhum"))
        row.addWidget(self.remove_btn)
        row.addStretch(1)
        self.progress = QProgressBar()
        self.progress.setRange(0, 0)
        self.progress.setFixedWidth(160)
        self.progress.setTextVisible(False)
        self.progress.setVisible(False)
        self.progress.setStyleSheet(f"QProgressBar {{ background: {C['panel2']}; border: none; border-radius: 4px; height: 8px; }}"
                                    f"QProgressBar::chunk {{ background: {C['accent']}; border-radius: 4px; }}")
        self.status = label("", "pageSub")
        self.cancel_btn = QPushButton("Cancelar")
        self.cancel_btn.setVisible(False)
        self.cancel_btn.clicked.connect(self.cancel)
        row.addWidget(self.status)
        row.addWidget(self.progress)
        row.addWidget(self.cancel_btn)
        self.root.addLayout(row)

        self.section("Registo")
        self.log = QPlainTextEdit()
        self.log.setReadOnly(True)
        self.log.setMaximumBlockCount(2000)
        self.root.addWidget(self.log, 1)
        self._validate()

    # --- validação --------------------------------------------------------
    def _validate(self):
        text = self.user.text()
        problem = validate_user(text)
        state = "true" if problem is None else ("false" if len(text.strip()) > 4 else "")
        self.user.setProperty("valid", state)
        self.user.style().unpolish(self.user)
        self.user.style().polish(self.user)
        self.hint.setText("ID válido." if problem is None else problem)
        self.hint.setStyleSheet(f"color: {C['green'] if problem is None else C['muted']};")
        self._set_enabled(problem is None and self.proc is None)

    def _set_enabled(self, on: bool):
        for b in self.buttons:
            b.setEnabled(on)
        self.remove_btn.setEnabled(on)

    def _write(self, text: str, color: str = C["text"]):
        fmt = QTextCharFormat()
        fmt.setForeground(QColor(color))
        cur = self.log.textCursor()
        cur.movePosition(cur.MoveOperation.End)
        cur.insertText(text if text.endswith("\n") else text + "\n", fmt)
        self.log.setTextCursor(cur)
        self.log.ensureCursorVisible()

    # --- execução ---------------------------------------------------------
    def emit_badge(self, key: str, name: str):
        if self.proc is not None:
            return
        user = self.user.text().strip()
        try:
            cmd = build_command(self.get_cfg(), user, key)
        except ValueError as exc:
            QMessageBox.warning(self, "Não é possível emitir", str(exc))
            return
        if key == "nenhum":
            ok = QMessageBox.question(self, "Retirar badge",
                                      f"Retirar o badge da conta\n{user}?\n\nA app passa a não mostrar nenhuma medalha.",
                                      QMessageBox.Yes | QMessageBox.Cancel, QMessageBox.Cancel)
            if ok != QMessageBox.Yes:
                return
        self._run(cmd, key, name, user)

    def _run(self, cmd: BadgeCommand, key: str, name: str, user: str):
        self.current = (name, user)
        self.started_at = time.monotonic()
        self._write(f"\n[{time.strftime('%H:%M:%S')}] {name} → {user}", C["accent"])
        self._write(f"$ {cmd.display()}", C["faint"])
        proc = QProcess(self)
        env = QProcessEnvironment.systemEnvironment()
        env.insert("PATH", cmd.path_env)
        env.insert("CARGO_TERM_COLOR", "never")
        proc.setProcessEnvironment(env)
        proc.setWorkingDirectory(cmd.cwd)
        proc.setProcessChannelMode(QProcess.MergedChannels)
        proc.readyReadStandardOutput.connect(self._on_output)
        proc.finished.connect(self._on_finished)
        proc.errorOccurred.connect(self._on_error)
        self.proc = proc
        self._busy(True, f"A emitir {name}…")
        proc.start(cmd.program, cmd.args)

    def _on_output(self):
        if self.proc:
            data = bytes(self.proc.readAllStandardOutput()).decode("utf-8", "replace")
            for line in data.splitlines():
                if line.strip():
                    self._write("  " + line, C["muted"])

    def _on_error(self, err):
        if err == QProcess.FailedToStart:
            self._done(False, "Não foi possível arrancar o cargo.")

    def _on_finished(self, code: int, status):
        self._on_output()
        if status == QProcess.CrashExit:
            self._done(False, "Cancelado." if code == 0 or code == 9 else f"Terminou de forma anormal ({code}).")
        else:
            self._done(code == 0, "Badge atualizado." if code == 0 else f"Falhou (código {code}). Vê o registo acima.")

    def _done(self, ok: bool, msg: str):
        if self.proc is None:
            return
        secs = time.monotonic() - self.started_at
        name, user = self.current or ("", "")
        self._write(f"{'✓' if ok else '✗'} {msg} ({secs:.1f} s)", C["green"] if ok else C["red"])
        self.proc.deleteLater()
        self.proc = None
        self._busy(False, ("✓ " if ok else "✗ ") + (f"{name} → {user[:18]}…" if ok else msg))
        self.status.setStyleSheet(f"color: {C['green'] if ok else C['red']};")
        self.finished.emit(ok, f"{name} → {user}" if ok else msg)

    def cancel(self):
        if self.proc:
            self.proc.kill()

    def _busy(self, busy: bool, text: str):
        self.status.setStyleSheet("")
        self.status.setText(text)
        self.progress.setVisible(busy)
        self.cancel_btn.setVisible(busy)
        self.user.setReadOnly(busy)
        self._validate() if not busy else self._set_enabled(False)
