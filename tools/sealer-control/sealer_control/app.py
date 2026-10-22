"""Janela principal: barra lateral, estado global, temporizadores, bandeja e notificações."""
from __future__ import annotations

import shutil
import subprocess
import sys
from typing import Any

from PySide6.QtCore import QRectF, QSize, Qt, QTimer
from PySide6.QtGui import QAction, QColor, QFont, QIcon, QLinearGradient, QPainter, QPainterPath, QPixmap
from PySide6.QtWidgets import (QApplication, QButtonGroup, QCheckBox, QDialog, QDialogButtonBox, QDoubleSpinBox,
                               QFileDialog, QFormLayout, QFrame, QHBoxLayout, QLabel, QLineEdit, QMainWindow, QMenu,
                               QMessageBox, QPushButton, QScrollArea, QSpinBox, QStackedWidget, QSystemTrayIcon,
                               QVBoxLayout, QWidget)

from . import __version__, aws, config, services
from .page_badges import BadgesPage
from .page_relay import RelayPage
from .page_site import SitePage
from .tasks import run_async
from .theme import C, QSS
from .widgets import Dot, StatusPill, label

AWS_KEYS = ("region", "profile", "namespace", "instance_id")


def make_icon(tone_color: str | None = None) -> QIcon:
    """Ícone da app (escudo roxo); com tone_color desenha um ponto de estado."""
    icon = QIcon()
    for size in (16, 24, 32, 64, 128, 256):
        pm = QPixmap(size, size)
        pm.fill(Qt.transparent)
        p = QPainter(pm)
        p.setRenderHint(QPainter.Antialiasing)
        s = size
        grad = QLinearGradient(0, 0, s, s)
        grad.setColorAt(0, QColor("#a99bff"))
        grad.setColorAt(1, QColor("#5b46d6"))
        p.setPen(Qt.NoPen)
        p.setBrush(grad)
        p.drawRoundedRect(QRectF(0, 0, s, s), s * 0.24, s * 0.24)
        shield = QPainterPath()
        shield.moveTo(s * 0.5, s * 0.18)
        shield.lineTo(s * 0.76, s * 0.28)
        shield.quadTo(s * 0.76, s * 0.66, s * 0.5, s * 0.82)
        shield.quadTo(s * 0.24, s * 0.66, s * 0.24, s * 0.28)
        shield.closeSubpath()
        p.setBrush(QColor(255, 255, 255, 235))
        p.drawPath(shield)
        if tone_color:
            r = s * 0.34
            p.setBrush(QColor(C["bg"]))
            p.drawEllipse(QRectF(s - r - 1, s - r - 1, r + 1, r + 1))
            p.setBrush(QColor(tone_color))
            p.drawEllipse(QRectF(s - r + s * 0.04, s - r + s * 0.04, r - s * 0.08, r - s * 0.08))
        p.end()
        icon.addPixmap(pm)
    return icon


class Notifier:
    """Notifica só quando um estado muda (evita repetir o mesmo alerta)."""

    def __init__(self, tray: QSystemTrayIcon | None):
        self.tray = tray
        self.state: dict[str, str] = {}
        self.enabled = True

    def change(self, key: str, tone: str, title: str, body: str) -> None:
        prev = self.state.get(key)
        self.state[key] = tone
        if not self.enabled or prev == tone:
            return
        if prev is None and tone == "ok":
            return  # arranque normal, nada a dizer
        if tone == "ok":
            title = f"Recuperado: {title}"
        self.send(title, body, tone)

    def send(self, title: str, body: str, tone: str = "info") -> None:
        if not self.enabled:
            return
        if self.tray and self.tray.isVisible() and self.tray.supportsMessages():
            icon = QSystemTrayIcon.Critical if tone == "bad" else (QSystemTrayIcon.Warning if tone == "warn" else QSystemTrayIcon.Information)
            self.tray.showMessage(title, body, icon, 8000)
        elif shutil.which("notify-send"):
            urgency = "critical" if tone == "bad" else "normal"
            subprocess.Popen(["notify-send", "-a", "Sealer Control", "-u", urgency, title, body])


class SettingsDialog(QDialog):
    def __init__(self, cfg: dict[str, Any], parent: QWidget):
        super().__init__(parent)
        self.setWindowTitle("Definições · Sealer Control")
        self.setMinimumWidth(660)
        lay = QVBoxLayout(self)
        lay.setContentsMargins(22, 18, 22, 18)
        lay.setSpacing(10)
        self.w: dict[str, Any] = {}

        def section(title: str) -> QFormLayout:
            lay.addWidget(label(title.upper(), "sectionTitle"))
            form = QFormLayout()
            form.setLabelAlignment(Qt.AlignRight | Qt.AlignVCenter)
            form.setHorizontalSpacing(14)
            lay.addLayout(form)
            return form

        def cap(text: str) -> QLabel:
            lbl = label(text, "muted")
            lbl.setFixedWidth(140)
            lbl.setAlignment(Qt.AlignRight | Qt.AlignVCenter)
            return lbl

        def text(form, key, caption, browse: str | None = None):
            edit = QLineEdit(str(cfg.get(key, "")))
            self.w[key] = edit
            if browse:
                row = QHBoxLayout()
                row.addWidget(edit, 1)
                btn = QPushButton("Escolher")
                btn.setStyleSheet("padding: 7px 10px;")
                btn.clicked.connect(lambda: self._browse(edit, browse))
                row.addWidget(btn)
                form.addRow(cap(caption), row)
            else:
                form.addRow(cap(caption), edit)

        f = section("AWS")
        text(f, "region", "Região")
        text(f, "profile", "Perfil AWS CLI")
        text(f, "instance_id", "ID da instância")
        self.w["instance_id"].setPlaceholderText("vazio = descobrir automaticamente")
        text(f, "namespace", "Namespace do agente")
        spin = QSpinBox()
        spin.setRange(30, 600)
        spin.setSuffix(" s")
        spin.setValue(int(cfg["refresh_seconds"]))
        self.w["refresh_seconds"] = spin
        f.addRow(cap("Atualizar a cada"), spin)

        f = section("Site e relay")
        text(f, "site_url", "Site")
        text(f, "estado_url", "estado.json")
        spin = QSpinBox()
        spin.setRange(15, 600)
        spin.setSuffix(" s")
        spin.setValue(int(cfg["site_interval"]))
        self.w["site_interval"] = spin
        f.addRow(cap("Verificar a cada"), spin)
        dspin = QDoubleSpinBox()
        dspin.setRange(0.2, 10)
        dspin.setSingleStep(0.1)
        dspin.setSuffix(" s")
        dspin.setValue(float(cfg["slow_threshold"]))
        self.w["slow_threshold"] = dspin
        f.addRow(cap("Lento a partir de"), dspin)

        f = section("Badges")
        text(f, "repo_path", "Repositório", "dir")
        text(f, "badge_key", "Chave do emissor", "file")
        text(f, "badge_db", "Base de dados", "file")

        f = section("Geral")
        for key, caption in (("notifications", "Notificações no ambiente de trabalho"),
                             ("minimize_to_tray", "Fechar a janela mantém a app na bandeja")):
            box = QCheckBox(caption)
            box.setChecked(bool(cfg[key]))
            self.w[key] = box
            f.addRow(cap(""), box)

        lay.addWidget(label("As credenciais vêm do perfil do AWS CLI (~/.aws). Não escrevas Access Keys aqui.", "faint", wrap=True))
        buttons = QDialogButtonBox(QDialogButtonBox.Save | QDialogButtonBox.Cancel)
        buttons.button(QDialogButtonBox.Save).setObjectName("primary")
        buttons.button(QDialogButtonBox.Save).setText("Guardar")
        buttons.button(QDialogButtonBox.Cancel).setText("Cancelar")
        buttons.accepted.connect(self._accept)
        buttons.rejected.connect(self.reject)
        lay.addWidget(buttons)
        self.result_cfg: dict[str, Any] | None = None

    def _browse(self, edit: QLineEdit, kind: str):
        start = config.expand(edit.text()) or str(config.Path.home())
        path = (QFileDialog.getExistingDirectory(self, "Escolher pasta", start) if kind == "dir"
                else QFileDialog.getOpenFileName(self, "Escolher ficheiro", start)[0])
        if path:
            edit.setText(path)

    def _accept(self):
        out: dict[str, Any] = {}
        for key, w in self.w.items():
            if isinstance(w, QLineEdit):
                out[key] = w.text().strip()
            elif isinstance(w, QCheckBox):
                out[key] = w.isChecked()
            else:
                out[key] = w.value()
        if not out["region"] or " " in out["region"]:
            QMessageBox.warning(self, "Definições", "Indica uma região AWS válida (ex.: eu-north-1).")
            return
        for key in ("site_url", "estado_url"):
            if not out[key].startswith("https://"):
                QMessageBox.warning(self, "Definições", f"O endereço {out[key]!r} tem de começar por https://")
                return
        out["profile"] = out["profile"] or "default"
        out["namespace"] = out["namespace"] or "Sealer/Relay"
        self.result_cfg = out
        self.accept()


class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        self.cfg = config.load_config()
        self.monitor: aws.AWSMonitor | None = None
        self.aws_busy = self.site_busy = False
        self.quitting = False
        self.setWindowTitle("Sealer Control")
        self.setWindowIcon(make_icon())
        self.resize(1240, 860)
        self.setMinimumSize(980, 640)

        central = QWidget()
        outer = QHBoxLayout(central)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)
        outer.addWidget(self._build_sidebar())
        right = QVBoxLayout()
        right.setContentsMargins(0, 0, 0, 0)
        right.setSpacing(0)
        right.addWidget(self._build_topbar())
        self.stack = QStackedWidget()
        right.addWidget(self.stack, 1)
        outer.addLayout(right, 1)
        self.setCentralWidget(central)

        self.relay = RelayPage()
        self.site = SitePage()
        self.badges = BadgesPage(lambda: self.cfg)
        for page in (self.relay, self.site, self.badges):
            scroll = QScrollArea()
            scroll.setWidgetResizable(True)
            scroll.setWidget(page)
            self.stack.addWidget(scroll)
        self.nav_group.idClicked.connect(self.stack.setCurrentIndex)

        self.relay.refresh_btn.clicked.connect(lambda: self.refresh_aws(force=True))
        self.site.check_btn.clicked.connect(self.check_site)
        self.badges.finished.connect(self._badge_done)

        self._build_tray()
        self.notifier = Notifier(self.tray)
        self._apply_cfg()

        self.aws_timer = QTimer(self, timeout=self.refresh_aws)
        self.site_timer = QTimer(self, timeout=self.check_site)
        self._restart_timers()
        QTimer.singleShot(300, self.check_site)
        QTimer.singleShot(600, self.refresh_aws)

    # --- construção -------------------------------------------------------
    def _build_sidebar(self) -> QWidget:
        side = QFrame()
        side.setObjectName("sidebar")
        side.setFixedWidth(230)
        lay = QVBoxLayout(side)
        lay.setContentsMargins(16, 22, 16, 16)
        lay.setSpacing(4)
        brand = QHBoxLayout()
        logo = label()
        logo.setPixmap(make_icon().pixmap(34, 34))
        names = QVBoxLayout()
        names.setSpacing(0)
        names.addWidget(label("SEALER", "brand"))
        names.addWidget(label("CONTROL", "brandSub"))
        brand.addWidget(logo)
        brand.addLayout(names)
        brand.addStretch(1)
        lay.addLayout(brand)
        lay.addSpacing(24)

        self.nav_group = QButtonGroup(self)
        self.nav_dots: list[Dot] = []
        for i, (icon, text) in enumerate((("◉", "Relay AWS"), ("◎", "Site"), ("✦", "Badges"))):
            row = QHBoxLayout()
            btn = QPushButton(f"{icon}   {text}")
            btn.setObjectName("navButton")
            btn.setCheckable(True)
            btn.setChecked(i == 0)
            btn.setCursor(Qt.PointingHandCursor)
            self.nav_group.addButton(btn, i)
            dot = Dot(7, "idle")
            self.nav_dots.append(dot)
            row.addWidget(btn, 1)
            if i < 2:
                row.addWidget(dot)
            lay.addLayout(row)
        lay.addStretch(1)
        settings = QPushButton("⚙   Definições")
        settings.setObjectName("navButton")
        settings.setCursor(Qt.PointingHandCursor)
        settings.clicked.connect(self.open_settings)
        lay.addWidget(settings)
        lay.addWidget(label(f"v{__version__}", "faint"))
        return side

    def _build_topbar(self) -> QWidget:
        bar = QFrame()
        bar.setStyleSheet(f"background: {C['bg']}; border-bottom: 1px solid {C['border']};")
        lay = QHBoxLayout(bar)
        lay.setContentsMargins(28, 12, 28, 12)
        lay.setSpacing(8)
        self.global_dot = Dot(10, "idle")
        self.global_text = label("A iniciar…")
        self.global_text.setStyleSheet("font-weight: 700; border: none;")
        lay.addWidget(self.global_dot)
        lay.addWidget(self.global_text)
        lay.addStretch(1)
        self.pill_site = StatusPill("Site")
        self.pill_relay = StatusPill("Relay")
        self.pill_aws = StatusPill("AWS")
        self.pill_lat = StatusPill("Envio —")
        for p in (self.pill_site, self.pill_relay, self.pill_aws, self.pill_lat):
            lay.addWidget(p)
        self.tones: dict[str, str] = {}
        return bar

    def _build_tray(self):
        self.tray = None
        if not QSystemTrayIcon.isSystemTrayAvailable():
            return
        self.tray = QSystemTrayIcon(make_icon(), self)
        self.tray.setToolTip("Sealer Control")
        menu = QMenu()
        menu.addAction(QAction("Abrir", self, triggered=self.show_window))
        menu.addAction(QAction("Verificar agora", self, triggered=lambda: (self.check_site(), self.refresh_aws(force=True))))
        menu.addSeparator()
        menu.addAction(QAction("Sair", self, triggered=self.quit))
        self.tray.setContextMenu(menu)
        self.tray.activated.connect(lambda reason: self.show_window() if reason == QSystemTrayIcon.Trigger else None)
        self.tray.show()
        self._tray_menu = menu

    # --- configuração -----------------------------------------------------
    def _apply_cfg(self):
        self.notifier.enabled = bool(self.cfg["notifications"])
        self.site.set_threshold(float(self.cfg["slow_threshold"]), self.cfg["site_url"])

    def _restart_timers(self):
        self.aws_timer.start(int(self.cfg["refresh_seconds"]) * 1000)
        self.site_timer.start(int(self.cfg["site_interval"]) * 1000)

    def open_settings(self):
        dlg = SettingsDialog(self.cfg, self)
        if dlg.exec() != QDialog.Accepted or not dlg.result_cfg:
            return
        aws_changed = any(dlg.result_cfg.get(k) != self.cfg.get(k) for k in AWS_KEYS)
        self.cfg.update(dlg.result_cfg)
        try:
            config.save_config(self.cfg)
        except OSError as exc:
            QMessageBox.warning(self, "Definições", f"Não foi possível guardar: {exc}")
        if aws_changed:
            self.monitor = None
        self._apply_cfg()
        self._restart_timers()
        self.check_site()
        self.refresh_aws(force=aws_changed)

    # --- AWS --------------------------------------------------------------
    def refresh_aws(self, force: bool = False):
        if self.aws_busy:
            return
        if aws.boto3 is None:
            self.relay.show_error("A biblioteca boto3 não está instalada (pip install boto3).")
            self._set_tone("aws", "bad", "AWS: sem boto3")
            return
        self.aws_busy = True
        self.relay.set_busy(True)
        cfg = dict(self.cfg)
        monitor = self.monitor

        def job():
            m = monitor or aws.AWSMonitor(cfg["region"], cfg["profile"], cfg["namespace"], cfg["instance_id"])
            return m, m.snapshot(force)

        run_async(job, self._aws_ok, self._aws_err)

    def _aws_ok(self, result):
        self.monitor, data = result
        self.aws_busy = False
        self.relay.set_busy(False)
        self.relay.show_data(data)
        tone = data["tone"]
        first = next((a for a in data["alerts"] if a[0] == tone), None)
        text = {"ok": "AWS OK", "warn": "AWS: aviso", "bad": "AWS: alerta"}[tone]
        self._set_tone("aws", tone, text)
        self.nav_dots[0].set_tone(tone)
        self.notifier.change("aws", tone, first[1] if first else "Relay AWS", first[2] if first else "Tudo normal.")

    def _aws_err(self, msg: str):
        self.aws_busy = False
        self.monitor = None
        self.relay.set_busy(False)
        msg = aws.friendly_error(msg, self.cfg.get("profile", "default"))
        self.relay.show_error(msg)
        self._set_tone("aws", "bad", "AWS: sem ligação")
        self.nav_dots[0].set_tone("bad")
        self.notifier.change("aws", "bad", "Sem ligação à AWS", msg[:200])

    # --- Site e estado.json -------------------------------------------------
    def check_site(self):
        if self.site_busy:
            return
        self.site_busy = True
        self.site.set_busy(True)
        url, slow, estado = self.cfg["site_url"], float(self.cfg["slow_threshold"]), self.cfg["estado_url"]

        def job():
            site = services.check_site(url, slow)
            try:
                lat, err = services.interpret_estado(services.fetch_estado(estado)), ""
            except Exception as exc:
                lat, err = None, str(exc)
            return site, lat, err

        run_async(job, self._site_ok, self._site_err)

    def _site_ok(self, result):
        site, lat, err = result
        self.site_busy = False
        self.site.set_busy(False)
        self.site.show_result(site, int(self.cfg["site_interval"]))
        self._set_tone("site", site["tone"], f"Site: {site['title'].lower()}")
        self.nav_dots[1].set_tone(site["tone"])
        self.notifier.change("site", site["tone"], f"sealer.pt — {site['title']}", site["detail"])

        self.relay.show_latency(lat, err)
        if lat is None:
            self._set_tone("relay", "bad", "Relay: sem estado")
            self.pill_lat.set("Envio —", "idle")
            self.notifier.change("relay", "bad", "Relay sem estado.json", err[:200])
        else:
            tone = lat["tone"]
            self._set_tone("relay", tone, "Relay ativo" if tone == "ok" else ("Relay lento" if tone == "warn" else "Relay parado?"))
            ida = lat["ida"]
            self.pill_lat.set(f"Envio {ida:.0f} ms" if ida is not None else "Envio —", "idle" if ida is None else tone)
            self.notifier.change("relay", tone, "Relay", "estado.json atualizado" if tone == "ok"
                                 else f"estado.json parado {services.age_text(lat['updated_age'])}")

    def _site_err(self, msg: str):
        self.site_busy = False
        self.site.set_busy(False)

    # --- estado global ------------------------------------------------------
    def _set_tone(self, key: str, tone: str, text: str):
        self.tones[key] = tone
        {"site": self.pill_site, "relay": self.pill_relay, "aws": self.pill_aws}[key].set(text, tone)
        tones = set(self.tones.values())
        overall = "bad" if "bad" in tones else ("warn" if "warn" in tones else "ok")
        self.global_dot.set_tone(overall)
        self.global_text.setText({"ok": "Tudo operacional", "warn": "Há avisos", "bad": "Atenção necessária"}[overall])
        if self.tray:
            self.tray.setIcon(make_icon(None if overall == "ok" else C["yellow" if overall == "warn" else "red"]))
            self.tray.setToolTip(f"Sealer Control — {self.global_text.text()}")

    def _badge_done(self, ok: bool, text: str):
        if not self.isActiveWindow():
            self.notifier.send("Badge atribuído" if ok else "Falha ao atribuir badge", text, "ok" if ok else "bad")

    # --- janela -------------------------------------------------------------
    def show_window(self):
        self.showNormal()
        self.raise_()
        self.activateWindow()

    def closeEvent(self, event):
        if not self.quitting and self.tray and self.cfg["minimize_to_tray"]:
            event.ignore()
            self.hide()
            if not getattr(self, "_told_tray", False):
                self._told_tray = True
                self.notifier.send("Sealer Control continua ativo", "A monitorização segue na bandeja. Usa Sair no menu para fechar.")
            return
        if self.badges.proc is not None:
            if QMessageBox.question(self, "Sair", "Há um badge a ser emitido. Sair mesmo assim?") != QMessageBox.Yes:
                event.ignore()
                return
            self.badges.cancel()
        event.accept()
        QApplication.quit()

    def quit(self):
        self.quitting = True
        self.close()


def main() -> int:
    QApplication.setApplicationName("Sealer Control")
    QApplication.setDesktopFileName("sealer-control")
    app = QApplication(sys.argv)
    app.setQuitOnLastWindowClosed(False)
    app.setStyleSheet(QSS)
    font = QFont()
    font.setPointSizeF(10)
    app.setFont(font)
    win = MainWindow()
    win.show()
    return app.exec()
