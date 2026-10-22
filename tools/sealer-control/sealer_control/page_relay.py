"""Página Relay AWS: instância, latência real, métricas, custos e alertas."""
from __future__ import annotations

import time
from typing import Any

from PySide6.QtWidgets import QGridLayout, QHBoxLayout, QListWidget, QListWidgetItem, QPushButton, QVBoxLayout, QWidget
from PySide6.QtGui import QColor

from .services import age_text
from .theme import C, TONE
from .widgets import Card, ChartCard, Dot, MetricCard, label


def fmt_bytes(v: float | None) -> str:
    if v is None:
        return "—"
    for unit in ("B", "KB", "MB", "GB"):
        if abs(v) < 1024:
            return f"{v:.1f} {unit}"
        v /= 1024
    return f"{v:.1f} TB"


def fmt_ms(v: float | None) -> str:
    return "—" if v is None else f"{v:,.0f}\u00a0ms".replace(",", "\u202f")


class Page(QWidget):
    def __init__(self, title: str, subtitle: str):
        super().__init__()
        self.setObjectName("page")
        self.root = QVBoxLayout(self)
        self.root.setContentsMargins(28, 22, 28, 22)
        self.root.setSpacing(14)
        head = QHBoxLayout()
        titles = QVBoxLayout()
        titles.setSpacing(2)
        titles.addWidget(label(title, "pageTitle"))
        self.subtitle = label(subtitle, "pageSub")
        titles.addWidget(self.subtitle)
        head.addLayout(titles)
        head.addStretch(1)
        self.actions = QHBoxLayout()
        self.actions.setSpacing(8)
        head.addLayout(self.actions)
        self.root.addLayout(head)

    def section(self, text: str):
        self.root.addWidget(label(text.upper(), "sectionTitle"))


class RelayPage(Page):
    def __init__(self):
        super().__init__("Relay AWS", "Instância EC2, latência real, recursos e custos")
        self.refresh_btn = QPushButton("↻  Atualizar")
        self.refresh_btn.setObjectName("primary")
        self.actions.addWidget(self.refresh_btn)

        # Faixa da instância
        banner = Card(padding=14)
        row = QHBoxLayout()
        self.inst_dot = Dot(10)
        self.inst_name = label("A ligar à AWS…", "cardValue")
        self.inst_name.setStyleSheet("font-size: 13pt;")
        row.addWidget(self.inst_dot)
        row.addWidget(self.inst_name)
        row.addStretch(1)
        self.conn = label("", "faint")
        row.addWidget(self.conn)
        banner.lay.addLayout(row)
        self.inst_details = label("Configura o perfil AWS nas Definições se isto não mudar.", "cardNote", wrap=True)
        banner.lay.addWidget(self.inst_details)
        self.root.addWidget(banner)

        # Latência real (estado.json)
        self.section("Latência das mensagens · estado.json do relay")
        lat = QHBoxLayout()
        lat.setSpacing(10)
        self.c_ida = MetricCard("Envio (mediana)", "—", "telemóvel → relay")
        self.c_entrega = MetricCard("Entrega (mediana)", "—", "relay → destinatário")
        self.c_total = MetricCard("Entre telemóveis", "—", "só com entregas da última hora")
        for c in (self.c_ida, self.c_entrega, self.c_total):
            lat.addWidget(c)
        self.root.addLayout(lat)

        # Métricas
        self.section("Recursos")
        grid = QGridLayout()
        grid.setSpacing(10)
        self.cards = {
            "health": MetricCard("Saúde EC2", "—", "status checks"),
            "cpu": MetricCard("CPU", "—", "CloudWatch · AWS/EC2"),
            "memory": MetricCard("RAM", "—", "agente Sealer"),
            "disk": MetricCard("Disco", "—", "agente Sealer"),
            "connections": MetricCard("Conexões TCP", "—", "não são utilizadores"),
            "relay": MetricCard("Serviço relay", "—", "systemd"),
            "errors": MetricCard("Erros", "—", "último minuto"),
            "cost": MetricCard("Custo do mês", "—", "Cost Explorer"),
        }
        for i, card in enumerate(self.cards.values()):
            grid.addWidget(card, i // 4, i % 4)
        self.root.addLayout(grid)

        charts = QGridLayout()
        charts.setSpacing(10)
        self.ch_cpu = ChartCard("CPU · 60 min", "%", C["blue"], 100)
        self.ch_mem = ChartCard("RAM · 60 min", "%", C["purple"], 100)
        self.ch_disk = ChartCard("Disco · 60 min", "%", C["yellow"], 100)
        self.ch_conn = ChartCard("Conexões TCP · 60 min", "", C["green"])
        for ch in (self.ch_cpu, self.ch_mem, self.ch_disk):
            ch.chart.thresholds = [(90, "bad")]
        for i, ch in enumerate((self.ch_cpu, self.ch_mem, self.ch_disk, self.ch_conn)):
            charts.addWidget(ch, i // 2, i % 2)
        self.root.addLayout(charts, 1)

        self.section("Alertas")
        self.alerts = QListWidget()
        self.alerts.setMaximumHeight(130)
        self.root.addWidget(self.alerts)

    # --- AWS --------------------------------------------------------------
    def set_busy(self, busy: bool):
        self.refresh_btn.setEnabled(not busy)
        self.refresh_btn.setText("A consultar…" if busy else "↻  Atualizar")

    def show_error(self, msg: str):
        self.inst_dot.set_tone("bad")
        self.inst_name.setText("Sem ligação à AWS")
        self.inst_details.setText(msg[:400])
        self.cards["health"].set("Erro", "Verifica perfil, região e permissões", "bad")
        self._fill_alerts([("bad", "Falha ao consultar a AWS", msg[:300])])

    def show_data(self, d: dict[str, Any]):
        inst = d["instance"]
        running = inst["state"] == "running"
        self.inst_dot.set_tone(d["tone"])
        self.inst_name.setText(f"{inst['name']}  ·  {inst['id']}")
        native, custom = d["native"], d["custom"]
        rx, tx = native.get("network_in", {}).get("value"), native.get("network_out", {}).get("value")
        self.inst_details.setText(
            f"{inst['type']}  ·  {inst['az']}  ·  IP {inst['public_ip']}  ·  rede (5 min): ↓ {fmt_bytes(rx)}  ↑ {fmt_bytes(tx)}")
        self.conn.setText(f"{d['region']} · atualizado às {d['collected_at']} · {d['calls']} chamadas AWS nesta sessão")

        ok_checks = d.get("system_status") == "ok" and d.get("instance_status") == "ok"
        tone = "ok" if running and ok_checks else ("warn" if running else "bad")
        self.cards["health"].set({"ok": "Saudável", "warn": "A verificar", "bad": "Atenção"}[tone],
                                 f"sistema {d.get('system_status')} · instância {d.get('instance_status')}", tone)

        cpu = native.get("cpu", {}).get("value")
        self.cards["cpu"].set("—" if cpu is None else f"{cpu:.1f}%", "média 5 min · AWS/EC2",
                              "idle" if cpu is None else ("bad" if cpu >= 85 else "ok"))

        def pct(key: str, note: str):
            v = custom.get(key, {}).get("value")
            if v is None:
                self.cards[key].set("Sem agente", "Instala o agente na EC2", "idle")
            else:
                self.cards[key].set(f"{v:.1f}%", note, "bad" if v >= 90 else ("warn" if v >= 80 else "ok"))
        pct("memory", "RAM usada")
        pct("disk", "volume raiz")

        conns = custom.get("connections", {}).get("value")
        self.cards["connections"].set("Sem agente" if conns is None else f"{conns:.0f}",
                                      "TCP estabelecidas (máx. 1 min)" if conns is not None else "Instala o agente na EC2",
                                      "idle" if conns is None else "info")
        relay = custom.get("relay_up", {}).get("value")
        if relay is None:
            self.cards["relay"].set("Sem agente", "Instala o agente na EC2", "idle")
        else:
            self.cards["relay"].set("Ativo" if relay >= 0.5 else "Parado", "qt-relay (systemd)", "ok" if relay >= 0.5 else "bad")
        errors = custom.get("errors", {}).get("value")
        self.cards["errors"].set("Sem agente" if errors is None else f"{errors:.0f}",
                                 "último minuto, só a contagem" if errors is not None else "Instala o agente na EC2",
                                 "idle" if errors is None else ("bad" if errors > 0 else "ok"))

        cost = d.get("cost") or {}
        if cost.get("amount") is not None:
            age = age_text(time.time() - cost.get("ts", 0))
            self.cards["cost"].set(f"{cost['amount']:.2f} {cost.get('unit', '')}".strip(),
                                   f"desde o dia 1 · lido {age}", "warn" if cost["amount"] > 20 else "info")
        else:
            denied = "AccessDenied" in cost.get("error", "")
            self.cards["cost"].set("Indisponível", "Falta ce:GetCostAndUsage" if denied else "Cost Explorer desligado?", "idle")

        self.ch_cpu.chart.set_values(native.get("cpu", {}).get("points", []))
        self.ch_mem.chart.set_values(custom.get("memory", {}).get("points", []))
        self.ch_disk.chart.set_values(custom.get("disk", {}).get("points", []))
        self.ch_conn.chart.set_values(custom.get("connections", {}).get("points", []))
        self._fill_alerts(d["alerts"] or [("ok", "Sem alertas nas métricas disponíveis", "")])

    def _fill_alerts(self, alerts):
        self.alerts.clear()
        order = {"bad": 0, "warn": 1, "info": 2, "ok": 3}
        for tone, title, detail in sorted(alerts, key=lambda a: order.get(a[0], 9)):
            item = QListWidgetItem(f"●  {title}" + (f"  —  {detail}" if detail else ""))
            item.setForeground(QColor(TONE.get(tone, C["text"])))
            item.setToolTip(detail)
            self.alerts.addItem(item)

    # --- Latência ---------------------------------------------------------
    def show_latency(self, e: dict[str, Any] | None, error: str = ""):
        if e is None:
            for c in (self.c_ida, self.c_entrega, self.c_total):
                c.set("—", error[:120] or "estado.json indisponível", "idle")
            return
        if e["relay_stale"]:
            self.c_ida.set(fmt_ms(e["ida"]), f"estado.json parado {age_text(e['updated_age'])} — o relay está a correr?", "bad")
        else:
            self.c_ida.set(fmt_ms(e["ida"]), f"p95 {fmt_ms(e['ida_p95'])} · {e['ida_n']} amostras · última hora",
                           "idle" if e["ida"] is None else ("ok" if e["ida"] < 150 else "warn"))
        if e["entrega"] is None:
            self.c_entrega.set("—", "ainda sem entregas medidas", "idle")
        elif e["entrega_live"]:
            self.c_entrega.set(fmt_ms(e["entrega"]), f"{e['mensagens']} mensagens na última hora · p95 {fmt_ms(e['entrega_p95'])}",
                               "ok" if e["entrega"] < 1000 else "warn")
        else:
            self.c_entrega.set(fmt_ms(e["entrega"]), f"medição antiga ({age_text(e['entrega_age'])}) · sem mensagens na última hora",
                               "idle" if e["entrega_stale"] else "info")
        if e["total"] is not None:
            self.c_total.set(fmt_ms(e["total"]), "envio + entrega, medições recentes", "ok" if e["total"] < 1500 else "warn")
        else:
            note = "sem entregas recentes para somar"
            if e["site_total"] is not None:
                note += f" · o site mostra {fmt_ms(e['site_total'])}"
            self.c_total.set("—", note, "idle")
