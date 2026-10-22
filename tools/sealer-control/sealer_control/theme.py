"""Paleta e folha de estilos (QSS)."""

C = {
    "bg": "#0a101c",
    "sidebar": "#0d1524",
    "panel": "#111b2d",
    "panel2": "#17243a",
    "hover": "#1c2c47",
    "border": "#22324c",
    "text": "#edf3fc",
    "muted": "#8fa1bb",
    "faint": "#5b6c86",
    "accent": "#8b7bff",   # roxo Sealer
    "blue": "#67b7ff",
    "green": "#4dd4a0",
    "yellow": "#ffd166",
    "red": "#ff7272",
    "purple": "#b5a1ff",
}

TONE = {"ok": C["green"], "warn": C["yellow"], "bad": C["red"], "idle": C["faint"], "info": C["blue"]}

# Cores dos badges (sombra -> brilho), iguais às da app
BADGES = [
    ("founder", "Founder", "#8A5F12", "#FFE48C"),
    ("cofounder", "Co-founder", "#9E2E66", "#FFC2DD"),
    ("dev", "Dev", "#141414", "#8A8A8A"),
    ("tester", "Tester", "#0A6F9C", "#B5F1FF"),
    ("contributor", "Contributor", "#0D5E33", "#7FE3A8"),
]

FONT = '"Inter", "Noto Sans", "Ubuntu", "Cantarell", sans-serif'

QSS = f"""
* {{ font-family: {FONT}; font-size: 10pt; color: {C['text']}; }}
QMainWindow, QDialog, #page {{ background: {C['bg']}; }}
QToolTip {{ background: {C['panel2']}; color: {C['text']}; border: 1px solid {C['border']}; padding: 6px; }}

#sidebar {{ background: {C['sidebar']}; border-right: 1px solid {C['border']}; }}
#brand {{ font-size: 15pt; font-weight: 700; letter-spacing: 3px; }}
#brandSub {{ color: {C['faint']}; font-size: 8pt; letter-spacing: 2px; }}
#navButton {{
    text-align: left; padding: 11px 14px; border: none; border-radius: 10px;
    background: transparent; color: {C['muted']}; font-weight: 600;
}}
#navButton:hover {{ background: {C['hover']}; color: {C['text']}; }}
#navButton:checked {{ background: {C['panel2']}; color: {C['text']}; }}

#pageTitle {{ font-size: 18pt; font-weight: 700; }}
#pageSub, .muted, #muted {{ color: {C['muted']}; }}
#faint {{ color: {C['faint']}; font-size: 8.5pt; }}
#sectionTitle {{ color: {C['muted']}; font-size: 8pt; font-weight: 700; letter-spacing: 1.5px; }}

#card {{ background: {C['panel']}; border: 1px solid {C['border']}; border-radius: 14px; }}
#cardTitle {{ color: {C['muted']}; font-size: 8pt; font-weight: 700; letter-spacing: 1.2px; }}
#cardValue {{ font-size: 20pt; font-weight: 700; }}
#cardNote {{ color: {C['muted']}; font-size: 8.5pt; }}
#bigValue {{ font-size: 30pt; font-weight: 800; }}

#statusPill {{ background: {C['panel']}; border: 1px solid {C['border']}; border-radius: 15px; padding: 5px 12px; }}

QPushButton {{
    background: {C['panel2']}; border: 1px solid {C['border']}; border-radius: 9px;
    padding: 8px 14px; font-weight: 600;
}}
QPushButton:hover {{ background: {C['hover']}; }}
QPushButton:disabled {{ color: {C['faint']}; background: {C['panel']}; }}
QPushButton#primary {{ background: {C['accent']}; border-color: {C['accent']}; color: #ffffff; }}
QPushButton#primary:hover {{ background: #9d8fff; }}
QPushButton#primary:disabled {{ background: {C['panel2']}; border-color: {C['border']}; color: {C['faint']}; }}
QPushButton#danger {{ color: {C['red']}; }}
QPushButton#danger:hover {{ background: #3a1d27; }}

QLineEdit, QSpinBox, QDoubleSpinBox {{
    background: {C['panel2']}; border: 1px solid {C['border']}; border-radius: 8px;
    padding: 7px 9px; selection-background-color: {C['accent']};
}}
QLineEdit:focus, QSpinBox:focus, QDoubleSpinBox:focus {{ border-color: {C['accent']}; }}
QSpinBox::up-button, QSpinBox::down-button, QDoubleSpinBox::up-button, QDoubleSpinBox::down-button {{ width: 0px; border: none; }}
QLineEdit[valid="false"] {{ border-color: {C['red']}; }}
QLineEdit[valid="true"] {{ border-color: {C['green']}; }}
QCheckBox {{ color: {C['muted']}; spacing: 8px; }}

QPlainTextEdit, QListWidget {{
    background: {C['bg']}; border: 1px solid {C['border']}; border-radius: 10px; padding: 6px;
    font-family: "JetBrains Mono", "DejaVu Sans Mono", monospace; font-size: 9pt;
}}
QListWidget::item {{ padding: 5px 4px; border-bottom: 1px solid {C['panel']}; }}

QTabWidget::pane {{ border: none; }}
QScrollArea {{ border: none; background: transparent; }}
QScrollBar:vertical {{ background: transparent; width: 10px; margin: 2px; }}
QScrollBar::handle:vertical {{ background: {C['border']}; border-radius: 4px; min-height: 30px; }}
QScrollBar::add-line, QScrollBar::sub-line {{ height: 0; }}
QMessageBox {{ background: {C['panel']}; }}
"""
