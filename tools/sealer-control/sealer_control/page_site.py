"""Página Site: semáforo do sealer.pt, tempo de resposta e histórico."""
from __future__ import annotations

import time
from collections import deque
from typing import Any

from PySide6.QtGui import QColor
from PySide6.QtWidgets import QHBoxLayout, QListWidget, QListWidgetItem, QPushButton, QVBoxLayout

from .page_relay import Page
from .theme import C, TONE
from .widgets import Card, ChartCard, Dot, MetricCard, label


class SitePage(Page):
    def __init__(self):
        super().__init__("Site", "Disponibilidade e tempo de resposta do sealer.pt")
        self.check_btn = QPushButton("↻  Verificar agora")
        self.check_btn.setObjectName("primary")
        self.actions.addWidget(self.check_btn)

        self.samples: deque[float] = deque(maxlen=120)
        self.checks = 0
        self.ok_checks = 0
        self.last_tone: str | None = None

        hero = Card(padding=22)
        row = QHBoxLayout()
        row.setSpacing(18)
        self.big_dot = Dot(34, "idle")
        texts = QVBoxLayout()
        texts.setSpacing(2)
        self.state = label("A verificar…", "bigValue")
        self.detail = label("", "pageSub")
        self.url = label("", "faint")
        texts.addWidget(self.state)
        texts.addWidget(self.detail)
        texts.addWidget(self.url)
        row.addWidget(self.big_dot)
        row.addLayout(texts, 1)
        hero.lay.addLayout(row)
        self.root.addWidget(hero)

        stats = QHBoxLayout()
        stats.setSpacing(10)
        self.c_uptime = MetricCard("Disponibilidade", "—", "desde que abriste a app")
        self.c_avg = MetricCard("Resposta média", "—", "últimas verificações")
        self.c_last = MetricCard("Última verificação", "—", "")
        for c in (self.c_uptime, self.c_avg, self.c_last):
            stats.addWidget(c)
        self.root.addLayout(stats)

        self.chart = ChartCard("Tempo de resposta", "ms", C["accent"], left_caption="")
        self.root.addWidget(self.chart, 1)

        self.section("Histórico de mudanças")
        self.history = QListWidget()
        self.history.setMaximumHeight(160)
        self.root.addWidget(self.history)

    def set_threshold(self, slow_s: float, url: str):
        self.chart.chart.thresholds = [(slow_s * 1000, "warn")]
        self.url.setText(url)

    def set_busy(self, busy: bool):
        self.check_btn.setEnabled(not busy)

    def show_result(self, r: dict[str, Any], interval: int):
        self.checks += 1
        self.ok_checks += r["tone"] != "bad"
        self.big_dot.set_tone(r["tone"])
        self.state.setText(r["title"])
        self.state.setStyleSheet(f"color: {TONE[r['tone']]};")
        self.detail.setText(r["detail"])
        if r["ms"] is not None:
            self.samples.append(r["ms"])
        self.chart.chart.left_caption = f"últimas {len(self.samples)} verificações · a cada {interval} s"
        self.chart.chart.set_values(list(self.samples))

        up = 100 * self.ok_checks / self.checks
        self.c_uptime.set(f"{up:.1f}%", f"{self.ok_checks}/{self.checks} verificações OK",
                          "ok" if up >= 99 else ("warn" if up >= 95 else "bad"))
        if self.samples:
            avg = sum(self.samples) / len(self.samples)
            self.c_avg.set(f"{avg:.0f} ms", f"mín. {min(self.samples):.0f} · máx. {max(self.samples):.0f} ms", "info")
        self.c_last.set(time.strftime("%H:%M:%S"), f"próxima dentro de {interval} s", r["tone"])

        if r["tone"] != self.last_tone:
            item = QListWidgetItem(f"{time.strftime('%d/%m %H:%M:%S')}   ●  {r['title']}  —  {r['detail']}")
            item.setForeground(QColor(TONE[r["tone"]]))
            self.history.insertItem(0, item)
            while self.history.count() > 200:
                self.history.takeItem(self.history.count() - 1)
            self.last_tone = r["tone"]
