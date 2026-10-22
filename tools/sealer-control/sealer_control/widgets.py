"""Widgets reutilizáveis: cartões, indicador de estado e gráfico de linha."""
from __future__ import annotations

from PySide6.QtCore import QPointF, QRectF, Qt, QSize
from PySide6.QtGui import QColor, QLinearGradient, QPainter, QPainterPath, QPen
from PySide6.QtWidgets import QFrame, QHBoxLayout, QLabel, QSizePolicy, QVBoxLayout, QWidget

from .theme import C, TONE


def label(text: str = "", name: str | None = None, wrap: bool = False) -> QLabel:
    lbl = QLabel(text)
    if name:
        lbl.setObjectName(name)
    lbl.setWordWrap(wrap)
    return lbl


class Card(QFrame):
    def __init__(self, parent: QWidget | None = None, padding: int = 16):
        super().__init__(parent)
        self.setObjectName("card")
        self.lay = QVBoxLayout(self)
        self.lay.setContentsMargins(padding, padding - 2, padding, padding - 2)
        self.lay.setSpacing(6)


class Dot(QWidget):
    """Círculo de estado com halo suave."""

    def __init__(self, size: int = 12, tone: str = "idle", parent: QWidget | None = None):
        super().__init__(parent)
        self._size = size
        self._color = QColor(TONE.get(tone, tone))
        self.setFixedSize(size + 8, size + 8)

    def set_tone(self, tone: str) -> None:
        self._color = QColor(TONE.get(tone, tone))
        self.update()

    def paintEvent(self, _event):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        p.setPen(Qt.NoPen)
        halo = QColor(self._color)
        halo.setAlpha(55)
        p.setBrush(halo)
        p.drawEllipse(self.rect())
        p.setBrush(self._color)
        p.drawEllipse(4, 4, self._size, self._size)


class StatusPill(QFrame):
    def __init__(self, text: str, parent: QWidget | None = None):
        super().__init__(parent)
        self.setObjectName("statusPill")
        lay = QHBoxLayout(self)
        lay.setContentsMargins(6, 2, 10, 2)
        lay.setSpacing(4)
        self.dot = Dot(8)
        self.text = label(text)
        lay.addWidget(self.dot)
        lay.addWidget(self.text)

    def set(self, text: str, tone: str) -> None:
        self.text.setText(text)
        self.dot.set_tone(tone)


class MetricCard(Card):
    def __init__(self, title: str, value: str = "—", note: str = "", tone: str = "info"):
        super().__init__(padding=15)
        self.setMinimumHeight(108)
        head = QHBoxLayout()
        head.setSpacing(6)
        self.dot = Dot(7, "idle")
        head.addWidget(label(title.upper(), "cardTitle"))
        head.addStretch(1)
        head.addWidget(self.dot)
        self.value = label(value, "cardValue")
        self.note = label(note, "cardNote", wrap=True)
        self.lay.addLayout(head)
        self.lay.addWidget(self.value)
        self.lay.addWidget(self.note)
        self.lay.addStretch(1)
        self.set(value, note, tone)

    def set(self, value: str, note: str | None = None, tone: str = "info") -> None:
        self.value.setText(value)
        if note is not None:
            self.note.setText(note)
        color = TONE.get(tone, C["text"])
        self.value.setStyleSheet(f"color: {color};")
        self.dot.set_tone(tone)


class Sparkline(QWidget):
    """Gráfico de linha leve, desenhado com QPainter (sem QtCharts)."""

    def __init__(self, unit: str = "%", color: str = C["blue"], fixed_max: float | None = None,
                 left_caption: str = "há 60 min", parent: QWidget | None = None):
        super().__init__(parent)
        self.unit = unit
        self.color = QColor(color)
        self.fixed_max = fixed_max
        self.left_caption = left_caption
        self.values: list[float] = []
        self.thresholds: list[tuple[float, str]] = []
        self.setMinimumHeight(110)
        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)

    def sizeHint(self) -> QSize:
        return QSize(300, 130)

    def set_values(self, values: list[float]) -> None:
        self.values = [float(v) for v in values if v is not None]
        self.update()

    def fmt(self, v: float) -> str:
        if self.unit == "ms":
            return f"{v:.0f} ms"
        if self.unit == "%":
            return f"{v:.1f}%"
        return f"{v:.0f}{self.unit}"

    def paintEvent(self, _event):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        w, h = self.width(), self.height()
        l, r, t, b = 6, 6, 10, 20
        pw, ph = w - l - r, h - t - b

        grid = QPen(QColor(C["border"]), 1, Qt.DashLine)
        p.setPen(grid)
        for i in range(3):
            y = t + ph * i / 2
            p.drawLine(QPointF(l, y), QPointF(w - r, y))

        p.setPen(QColor(C["faint"]))
        font = p.font()
        font.setPointSizeF(8)
        p.setFont(font)
        if not self.values:
            p.drawText(QRectF(0, 0, w, h), Qt.AlignCenter, "À espera de dados")
            return

        vmax = max(self.values)
        top = self.fixed_max if self.fixed_max else (vmax * 1.2 if vmax > 0 else 1.0)
        top = max(top, vmax)
        n = len(self.values)

        def pt(i: int, v: float) -> QPointF:
            x = l + (pw * i / (n - 1) if n > 1 else pw)
            return QPointF(x, t + ph - (min(v, top) / top) * ph)

        for value, tone in self.thresholds:
            if value <= top:
                pen = QPen(QColor(TONE.get(tone, tone)), 1, Qt.DotLine)
                p.setPen(pen)
                y = t + ph - value / top * ph
                p.drawLine(QPointF(l, y), QPointF(w - r, y))

        pts = [pt(i, v) for i, v in enumerate(self.values)]
        line = QPainterPath(pts[0])
        for q in pts[1:]:
            line.lineTo(q)
        if n > 1:
            area = QPainterPath(line)
            area.lineTo(pts[-1].x(), t + ph)
            area.lineTo(pts[0].x(), t + ph)
            area.closeSubpath()
            grad = QLinearGradient(0, t, 0, t + ph)
            c1 = QColor(self.color); c1.setAlpha(90)
            c2 = QColor(self.color); c2.setAlpha(0)
            grad.setColorAt(0, c1)
            grad.setColorAt(1, c2)
            p.fillPath(area, grad)
            p.setPen(QPen(self.color, 2))
            p.drawPath(line)
        p.setPen(Qt.NoPen)
        p.setBrush(self.color)
        p.drawEllipse(pts[-1], 3.5, 3.5)

        p.setPen(QColor(C["faint"]))
        p.drawText(QRectF(l, h - 16, pw / 2, 16), Qt.AlignLeft | Qt.AlignVCenter, self.left_caption)
        font.setBold(True)
        p.setFont(font)
        p.setPen(QColor(C["text"]))
        p.drawText(QRectF(l + pw / 2, h - 16, pw / 2, 16), Qt.AlignRight | Qt.AlignVCenter, self.fmt(self.values[-1]))


class ChartCard(Card):
    def __init__(self, title: str, unit: str, color: str, fixed_max: float | None = None,
                 left_caption: str = "há 60 min"):
        super().__init__(padding=14)
        self.lay.addWidget(label(title, "cardTitle"))
        self.chart = Sparkline(unit, color, fixed_max, left_caption)
        self.lay.addWidget(self.chart, 1)
