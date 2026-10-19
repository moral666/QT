#!/usr/bin/env python3
"""
Gera o esquema da arquitetura da SEALER, em português e em inglês:
docs/arquitetura.svg e docs/architecture.svg, na raiz do repositório QT
(a pasta acima de site/).

O texto é desenhado com os tipos de letra da marca (Manrope e Unbounded, em
src/assets/fonts) e convertido em contornos. Assim, o esquema fica igual em
qualquer sítio, incluindo o GitHub, sem depender das fontes de quem o vê.

Para mudar um texto, edite-o em CONTENT (nas duas línguas) e volte a correr o
script. Se um texto não couber no seu espaço, o script pára e diz qual é.

Uso, dentro da pasta site/:
  python3 scripts/architecture_diagram.py           escreve em ../docs/
  python3 scripts/architecture_diagram.py pasta/    escreve noutra pasta

Dependências: fonttools, brotli, uharfbuzz
  pip install fonttools brotli uharfbuzz
"""

import io
import sys
from pathlib import Path

import uharfbuzz as hb
from fontTools.pens.basePen import BasePen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent
FONTS = ROOT / "src" / "assets" / "fonts"
DEFAULT_DIR = ROOT.parent / "docs"

# ---------------------------------------------------------------------------
# Conteúdo
# ---------------------------------------------------------------------------

CONTENT = {
    "pt": {
        "file": "arquitetura.svg",
        "svg_title": "Arquitetura da SEALER",
        "eyebrow": "Projeto QT",
        "title": "Arquitetura",
        "lede": "A cifra acontece nos telemóveis. O relay só guarda e entrega envelopes fechados.",
        "phone": "Telemóvel",
        # Camadas: (título, subtítulo ou None, é o núcleo?)
        "android": [
            ("App SEALER", "Kotlin · Jetpack Compose", False),
            ("Ligações UniFFI", "Kotlin · libqt_ffi.so", False),
            ("Núcleo QT em Rust", "a cifra acontece aqui", True),
        ],
        "android_key": "Chave local: Android Keystore",
        "iphone": [
            ("App SEALER", "SwiftUI", False),
            ("Ligações UniFFI", "Swift · QtFFI.xcframework", False),
            ("Núcleo QT em Rust", "o mesmo do Android", True),
        ],
        "iphone_key": "Chave local: Secure Enclave",
        "server": "Servidor · AWS, Estocolmo",
        "relay": [
            ("HAProxy", "TLS com Let's Encrypt", False),
            ("Relay em Rust", "envelopes e anexos cifrados", False),
            ("Redis", "filas e pre-keys, só local", False),
        ],
        "relay_never": "Não vê o conteúdo nem quem envia",
        "link": "Noise_XX sobre wss://, com a chave do relay fixada nas apps",
        "core_eyebrow": "Em cada telemóvel",
        "core_title": "Núcleo QT em Rust",
        "core_lede": "Toda a criptografia vive aqui, nada é reimplementado nas apps. "
        "O mensageiro do terminal (cli/) usa o mesmo núcleo.",
        # Crates: (nome, função, o que tem)
        "crates": [
            ("core", "Criptografia", "X3DH e PQXDH (X25519 + ML-KEM-768) · Double Ratchet · Sealed Sender"),
            ("transport", "Ligação", "Noise_XX (X25519, ChaCha20-Poly1305) sobre WebSocket · chave do relay fixada"),
            ("storage", "Dados no aparelho", "SQLCipher · secure_delete · histórico e ratchet na mesma transação"),
            ("ffi", "Ponte para as apps", "UniFFI para Kotlin, Swift e Python · cifra dos anexos de imagem"),
        ],
        "site_eyebrow": "Fora do caminho das mensagens",
        "site_title": "Site sealer.pt",
        "site_sub": "Pasta site/ deste repositório",
        "site_facts": ["Cloudflare, Worker sealer", "Só ficheiros estáticos",
                       "Sem cookies, CSP estrita", "Não comunica com o relay"],
        "footer": "SEALER · protocolo QT · versão alpha",
        "legend_solid": "mensagens cifradas de ponta a ponta",
        "legend_dashed": "o mesmo núcleo, em detalhe",
        "description": (
            "Duas apps SEALER, Android e iPhone, usam o mesmo núcleo QT em Rust através de ligações UniFFI. "
            "A cifra acontece no núcleo, dentro de cada telemóvel. As apps trocam envelopes cifrados através "
            "de um relay na AWS, em Estocolmo (HAProxy com TLS, relay em Rust e Redis), por Noise_XX sobre wss "
            "com a chave do relay fixada nas apps. O relay nunca vê o conteúdo nem quem envia. O núcleo tem "
            "quatro partes: core, transport, storage e ffi. O site sealer.pt está à parte, na Cloudflare, e "
            "não comunica com o relay."
        ),
    },
    "en": {
        "file": "architecture.svg",
        "svg_title": "SEALER architecture",
        "eyebrow": "QT project",
        "title": "Architecture",
        "lede": "Encryption happens on the phones. The relay only stores and delivers sealed envelopes.",
        "phone": "Phone",
        "android": [
            ("SEALER app", "Kotlin · Jetpack Compose", False),
            ("UniFFI bindings", "Kotlin · libqt_ffi.so", False),
            ("QT core in Rust", "encryption happens here", True),
        ],
        "android_key": "Local key: Android Keystore",
        "iphone": [
            ("SEALER app", "SwiftUI", False),
            ("UniFFI bindings", "Swift · QtFFI.xcframework", False),
            ("QT core in Rust", "the same as on Android", True),
        ],
        "iphone_key": "Local key: Secure Enclave",
        "server": "Server · AWS, Stockholm",
        "relay": [
            ("HAProxy", "TLS with Let's Encrypt", False),
            ("Relay in Rust", "encrypted envelopes and files", False),
            ("Redis", "queues and pre-keys, local only", False),
        ],
        "relay_never": "Never sees content or the sender",
        "link": "Noise_XX over wss://, with the relay key pinned in the apps",
        "core_eyebrow": "On every phone",
        "core_title": "QT core in Rust",
        "core_lede": "All cryptography lives here; nothing is reimplemented in the apps. "
        "The terminal messenger (cli/) uses the same core.",
        "crates": [
            ("core", "Cryptography", "X3DH and PQXDH (X25519 + ML-KEM-768) · Double Ratchet · Sealed Sender"),
            ("transport", "Connection", "Noise_XX (X25519, ChaCha20-Poly1305) over WebSocket · pinned relay key"),
            ("storage", "On-device data", "SQLCipher · secure_delete · history and ratchet in one transaction"),
            ("ffi", "Bridge to the apps", "UniFFI for Kotlin, Swift and Python · image attachment encryption"),
        ],
        "site_eyebrow": "Outside the message path",
        "site_title": "sealer.pt website",
        "site_sub": "Folder site/ in this repository",
        "site_facts": ["Cloudflare, Worker sealer", "Static files only",
                       "No cookies, strict CSP", "Never talks to the relay"],
        "footer": "SEALER · QT protocol · alpha",
        "legend_solid": "end-to-end encrypted messages",
        "legend_dashed": "the same core, in detail",
        "description": (
            "Two SEALER apps, Android and iPhone, use the same QT core in Rust through UniFFI bindings. "
            "Encryption happens in the core, on each phone. The apps exchange encrypted envelopes through a "
            "relay on AWS in Stockholm (HAProxy with TLS, a Rust relay and Redis), using Noise_XX over wss with "
            "the relay key pinned in the apps. The relay never sees the content or the sender. The core has "
            "four parts: core, transport, storage and ffi. The sealer.pt website is separate, on Cloudflare, "
            "and never talks to the relay."
        ),
    },
}

# ---------------------------------------------------------------------------
# Aspeto (cores do site, src/styles.css)
# ---------------------------------------------------------------------------

PANEL = "#141726"
PANEL_DEEP = "#0d0f19"
MIST = "#a1a7bb"
ICE = "#eceef6"
SOFT = "#c9cdde"
DIM = "#767c95"
LILAC = "#c9b9f6"
VIOLET = "#6e48db"
COLD = "#5d86f0"
COLD_LIGHT = "#b9ccff"

# (tipo de letra, tamanho, peso, espaçamento entre letras em em, cor)
STYLES = {
    "eyebrow": ("text", 14, 500, 0.02, MIST),
    "title": ("display", 40, 300, -0.02, ICE),
    "lede": ("text", 17, 400, 0, MIST),
    "wordmark": ("display", 21, 500, 0.06, ICE),
    "domain": ("text", 13.5, 500, 0.01, MIST),
    "card": ("text", 24, 600, -0.01, ICE),
    "card-sub": ("text", 14, 400, 0, MIST),
    "layer": ("text", 16, 600, 0, ICE),
    "layer-sub": ("text", 13.5, 400, 0, MIST),
    "foot": ("text", 13.5, 500, 0, SOFT),
    "item": ("text", 15, 500, 0, SOFT),
    "pill": ("text", 13.5, 700, 0.03, COLD_LIGHT),
    "crate": ("text", 16, 600, 0, ICE),
    "caption": ("text", 14.5, 500, 0.01, COLD_LIGHT),
    "footer": ("text", 13.5, 400, 0.01, DIM),
}

W = 1100
M = 44  # margem exterior

# ---------------------------------------------------------------------------
# Texto em contornos
# ---------------------------------------------------------------------------


def num(value):
    s = f"{value:.1f}"
    if s.endswith(".0"):
        s = s[:-2]
    return "0" if s == "-0" else s


def short(value):
    """Número com uma casa decimal e o mínimo de caracteres (0.5 fica .5)."""
    s = num(value)
    if s.startswith("0."):
        return s[1:]
    if s.startswith("-0."):
        return "-" + s[2:]
    return s


def join_numbers(values):
    """Junta números como o SVG os aceita, sem espaços desnecessários: um sinal
    de menos já separa, e ".5" separa-se de um número que já tem ponto."""
    result, previous = "", None
    for v in values:
        s = short(v)
        if previous is not None and not s.startswith("-") and not (s.startswith(".") and "." in previous):
            result += " "
        result += s
        previous = s
    return result


class GlyphPen(BasePen):
    """Contorno de um glifo em comandos relativos, arredondados a uma casa decimal.

    As diferenças são calculadas entre pontos já arredondados, para os erros
    de arredondamento não se acumularem ao longo do contorno."""

    def __init__(self, scale):
        super().__init__(None)
        self.scale = scale
        self.parts = []
        self.current = (0.0, 0.0)
        self.start = (0.0, 0.0)

    def _point(self, p):
        return (round(p[0] * self.scale, 1), round(-p[1] * self.scale, 1))

    def _moveTo(self, p):
        x, y = self._point(p)
        x0, y0 = self.current
        self.parts.append(("m" if self.parts else "M") + join_numbers([x - x0, y - y0]))
        self.current = self.start = (x, y)

    def _lineTo(self, p):
        x, y = self._point(p)
        x0, y0 = self.current
        if y == y0:
            self.parts.append("h" + short(x - x0))
        elif x == x0:
            self.parts.append("v" + short(y - y0))
        else:
            self.parts.append("l" + join_numbers([x - x0, y - y0]))
        self.current = (x, y)

    def _qCurveToOne(self, p1, p2):
        x0, y0 = self.current
        (x1, y1), (x2, y2) = self._point(p1), self._point(p2)
        self.parts.append("q" + join_numbers([x1 - x0, y1 - y0, x2 - x0, y2 - y0]))
        self.current = (x2, y2)

    def _curveToOne(self, p1, p2, p3):
        x0, y0 = self.current
        pts = [self._point(p) for p in (p1, p2, p3)]
        self.parts.append("c" + join_numbers([v for x, y in pts for v in (x - x0, y - y0)]))
        self.current = pts[-1]

    def _closePath(self):
        self.parts.append("z")
        self.current = self.start

    def _endPath(self):
        self.current = self.start

    def path(self):
        return "".join(self.parts)


class Typeface:
    def __init__(self, path):
        font = TTFont(path)
        self.cmap = font.getBestCmap()
        font.flavor = None
        data = io.BytesIO()
        font.save(data)
        self.face = hb.Face(data.getvalue())
        self.upem = self.face.upem
        self._fonts = {}

    def font(self, weight):
        if weight not in self._fonts:
            font = hb.Font(self.face)
            font.set_variations({"wght": weight})
            self._fonts[weight] = font
        return self._fonts[weight]


TYPEFACES = {
    "text": Typeface(FONTS / "manrope-latin-var.woff2"),
    "display": Typeface(FONTS / "unbounded-latin-var.woff2"),
}


def shape(s, style):
    face_key, size, weight, tracking, _ = STYLES[style]
    face = TYPEFACES[face_key]
    missing = sorted({c for c in s if ord(c) not in face.cmap})
    if missing:
        sys.exit(f"O tipo de letra não tem {missing!r} (texto: {s!r}).")
    font = face.font(weight)
    buf = hb.Buffer()
    buf.add_str(s)
    buf.guess_segment_properties()
    hb.shape(font, buf, {"kern": True, "liga": True})
    scale = size / face.upem
    glyphs, x = [], 0.0
    infos, positions = buf.glyph_infos, buf.glyph_positions
    for i, (info, pos) in enumerate(zip(infos, positions)):
        glyphs.append((info.codepoint, x + pos.x_offset * scale, pos.y_offset * scale))
        x += pos.x_advance * scale
        if i < len(infos) - 1:
            x += tracking * size
    return glyphs, x


def measure(s, style):
    return shape(s, style)[1]


class Drawing:
    """Os elementos de um esquema: formas, degradês e glifos.

    Cada glifo é desenhado uma só vez, a um tamanho de referência, e depois
    reutilizado (<use>) em todos os sítios onde aparece, à escala certa."""

    GLYPH_SIZE = 100

    def __init__(self):
        self.out = []
        self.defs = []
        self.glyph_ids = {}

    def add(self, element):
        self.out.append(element)

    def glyph(self, face_key, weight, gid):
        key = (face_key, weight, gid)
        if key not in self.glyph_ids:
            face = TYPEFACES[face_key]
            pen = GlyphPen(self.GLYPH_SIZE / face.upem)
            face.font(weight).draw_glyph_with_pen(gid, pen)
            d = pen.path()
            self.glyph_ids[key] = f"g{len(self.glyph_ids):x}" if d else None
            if d:
                self.defs.append(f'<path id="{self.glyph_ids[key]}" d="{d}"/>')
        return self.glyph_ids[key]

    def x_gradient(self, x1, x2, c1, c2):
        """Degradê horizontal em coordenadas absolutas (funciona também em linhas
        retas, que não têm altura)."""
        id_ = f"lg{len(self.defs)}"
        self.defs.append(
            f'<linearGradient id="{id_}" gradientUnits="userSpaceOnUse" x1="{num(x1)}" y1="0" x2="{num(x2)}" y2="0">'
            f'<stop offset="0" stop-color="{c1}"/><stop offset="1" stop-color="{c2}"/></linearGradient>'
        )
        return id_


d = Drawing()


def text(x, y, s, style, anchor="start", fill=None, max_width=None):
    """Desenha s com a linha de base em y. Devolve a largura."""
    face_key, size, weight = STYLES[style][:3]
    glyphs, width = shape(s, style)
    if max_width is not None and width > max_width + 0.01:
        sys.exit(f"Não cabe: {s!r} mede {width:.0f}, o espaço é {max_width:.0f}.")
    if anchor == "middle":
        x -= width / 2
    elif anchor == "end":
        x -= width
    k = size / Drawing.GLYPH_SIZE
    uses = []
    for gid, gx, gy in glyphs:
        ref = d.glyph(face_key, weight, gid)
        if ref is None:  # espaço
            continue
        attrs = f' x="{num(gx / k)}"' if abs(gx) > 0.004 else ""
        if abs(gy) > 0.004:
            attrs += f' y="{num(-gy / k)}"'
        uses.append(f'<use href="#{ref}"{attrs}/>')
    d.add(
        f'<g fill="{fill or STYLES[style][4]}" transform="translate({num(x)} {num(y)}) scale({k:g})">'
        f'{"".join(uses)}</g>'
    )
    return width


# ---------------------------------------------------------------------------
# Formas
# ---------------------------------------------------------------------------


def bubble(x, y, w, h, r=22, r_tail=6):
    """Retângulo com a forma das janelas do site: o canto de baixo, à esquerda,
    é mais vivo, como o bico do balão do logótipo."""
    return (
        f"M{num(x + r)} {num(y)}H{num(x + w - r)}A{r} {r} 0 0 1 {num(x + w)} {num(y + r)}"
        f"V{num(y + h - r)}A{r} {r} 0 0 1 {num(x + w - r)} {num(y + h)}"
        f"H{num(x + r_tail)}A{r_tail} {r_tail} 0 0 1 {num(x)} {num(y + h - r_tail)}"
        f"V{num(y + r)}A{r} {r} 0 0 1 {num(x + r)} {num(y)}Z"
    )


def card(x, y, w, h, tone, dashed=False):
    path = bubble(x, y, w, h)
    color = COLD if tone == "cold" else LILAC
    if dashed:
        d.add(f'<path d="{path}" fill="url(#panel)" fill-opacity=".6"/>')
        d.add(f'<path d="{path}" fill="none" stroke="{LILAC}" stroke-opacity=".38" stroke-dasharray="5 5"/>')
        return
    d.add(f'<path d="{path}" fill="url(#panel)"/>')
    d.add(f'<path d="{path}" fill="url(#glow-{tone})"/>')
    d.add(f'<path d="{path}" fill="none" stroke="{color}" stroke-opacity="{".3" if tone == "cold" else ".2"}"/>')


def dot(cx, cy, tone):
    """O ponto dos rótulos: um dos três pontos do logótipo."""
    glow = {"violet": VIOLET, "cold": COLD, "white": "#c9cde0"}[tone]
    d.add(f'<circle cx="{num(cx)}" cy="{num(cy)}" r="6" fill="{glow}" fill-opacity=".75" filter="url(#blur)"/>')
    d.add(f'<circle cx="{num(cx)}" cy="{num(cy)}" r="5" fill="url(#dot-{tone})"/>')


def eyebrow(x, y, s, tone, max_width=None):
    dot(x + 5, y - 5, tone)
    text(x + 18, y, s, "eyebrow", max_width=None if max_width is None else max_width - 18)


def stroke_group(color, opacity, body):
    d.add(
        f'<g fill="none" stroke="{color}" stroke-opacity="{opacity}" stroke-width="1.5" '
        f'stroke-linecap="round" stroke-linejoin="round">{body}</g>'
    )


def lock(x, y, color, opacity=1.0):
    """Cadeado de 12 x 15, com o canto de cima, à esquerda, em (x, y)."""
    stroke_group(
        color, opacity,
        f'<rect x="{num(x)}" y="{num(y + 6)}" width="12" height="9" rx="2.2"/>'
        f'<path d="M{num(x + 3)} {num(y + 6)}V{num(y + 4)}a3 3 0 0 1 6 0V{num(y + 6)}"/>',
    )


def key(x, y, color, opacity=1.0):
    """Chave de 16 x 11, com o canto de cima, à esquerda, em (x, y)."""
    stroke_group(
        color, opacity,
        f'<circle cx="{num(x + 4)}" cy="{num(y + 5)}" r="3.6"/>'
        f'<path d="M{num(x + 7.6)} {num(y + 5)}H{num(x + 16)}M{num(x + 12.5)} {num(y + 5)}v3M{num(x + 15.2)} {num(y + 5)}v2.4"/>',
    )


def check(x, y, color):
    """Visto de 11 x 8, com o canto de cima, à esquerda, em (x, y)."""
    d.add(
        f'<path d="M{num(x)} {num(y + 4.2)}l3.4 3.4L{num(x + 11)} {num(y)}" fill="none" stroke="{color}" '
        f'stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>'
    )


def envelope(cx, cy):
    """Envelope selado (o mesmo desenho da janela "Remetente selado" do site)."""
    ox, oy = cx - 12, cy - 12
    stroke_group(
        LILAC, 1,
        f'<rect x="{num(ox + 3)}" y="{num(oy + 5.5)}" width="18" height="13" rx="2"/>'
        f'<path d="M{num(ox + 3)} {num(oy + 7.5)}L{num(ox + 12)} {num(oy + 13.5)}L{num(ox + 21)} {num(oy + 7.5)}"/>',
    )
    d.add(f'<circle cx="{num(ox + 12)}" cy="{num(oy + 15.6)}" r="2.9" fill="{VIOLET}" stroke="{LILAC}" stroke-width="1.2"/>')


def arrowhead(x, y, direction):
    k = 1 if direction == "right" else -1
    d.add(f'<path d="M{num(x)} {num(y)}L{num(x - 8 * k)} {num(y - 4.5)}L{num(x - 8 * k)} {num(y + 4.5)}Z" fill="{LILAC}"/>')


def link(x1, x2, y, c1, c2):
    """Ligação nos dois sentidos entre um telemóvel e o relay, com o envelope a meio."""
    cx = (x1 + x2) / 2
    r = 17
    a, b = x1 + 4, x2 - 4
    gradient = d.x_gradient(a, b, c1, c2)
    d.add(
        f'<path d="M{num(a + 6)} {num(y)}H{num(cx - r - 4)}M{num(cx + r + 4)} {num(y)}H{num(b - 6)}" '
        f'stroke="url(#{gradient})" stroke-width="1.6" fill="none"/>'
    )
    arrowhead(a, y, "left")
    arrowhead(b, y, "right")
    d.add(f'<circle cx="{num(cx)}" cy="{num(y)}" r="{r}" fill="{PANEL_DEEP}" stroke="{LILAC}" stroke-opacity=".45"/>')
    envelope(cx, y)


def connector(x, y1, y2):
    d.add(
        f'<path d="M{num(x)} {num(y1)}V{num(y2)}" stroke="{COLD}" stroke-opacity=".6" stroke-width="1.4" '
        f'stroke-dasharray="3 5" stroke-linecap="round"/>'
    )
    for y in (y1, y2):
        d.add(f'<circle cx="{num(x)}" cy="{num(y)}" r="3" fill="{COLD}"/>')


# ---------------------------------------------------------------------------
# Blocos
# ---------------------------------------------------------------------------

LAYER_H, LAYER_GAP, LAYERS_Y = 54, 14, 98


def stack_card(x, y, w, h, eyebrow_text, eyebrow_tone, title, layers, foot_icon, foot_text, tone):
    """Cartão com rótulo, título, três camadas empilhadas e uma linha de rodapé."""
    card(x, y, w, h, tone)
    eyebrow(x + 24, y + 38, eyebrow_text, eyebrow_tone, max_width=w - 48)
    text(x + 24, y + 76, title, "card", max_width=w - 48)
    lx, lw = x + 24, w - 48
    ly = y + LAYERS_Y
    for i, (name, sub, is_core) in enumerate(layers):
        color = COLD if is_core else LILAC
        d.add(
            f'<rect x="{num(lx)}" y="{num(ly)}" width="{num(lw)}" height="{LAYER_H}" rx="12" '
            f'fill="{color}" fill-opacity="{".11" if is_core else ".045"}" '
            f'stroke="{color}" stroke-opacity="{".55" if is_core else ".2"}"/>'
        )
        room = lw - 32 - (24 if is_core else 0)
        if sub:
            text(lx + 16, ly + 23, name, "layer", max_width=room)
            text(lx + 16, ly + 41, sub, "layer-sub", max_width=room)
        else:
            text(lx + 16, ly + LAYER_H / 2 + 5.5, name, "layer", max_width=room)
        if is_core:
            lock(lx + lw - 28, ly + LAYER_H / 2 - 8, COLD_LIGHT)
        if i < len(layers) - 1:
            d.add(f'<path d="M{num(x + w / 2)} {num(ly + LAYER_H)}V{num(ly + LAYER_H + LAYER_GAP)}" stroke="{LILAC}" stroke-opacity=".35"/>')
        ly += LAYER_H + LAYER_GAP
    # Rodapé do cartão
    rule_y = ly - LAYER_GAP + 18
    d.add(f'<path d="M{num(x + 24)} {num(rule_y)}H{num(x + w - 24)}" stroke="{LILAC}" stroke-opacity=".12"/>')
    base = rule_y + 25
    if foot_icon == "key":
        key(x + 24, base - 9.5, LILAC, 0.9)
    else:
        lock(x + 26, base - 13, LILAC, 0.9)
    text(x + 48, base, foot_text, "foot", max_width=w - 72)
    return base + 22 - y  # altura necessária


def core_panel(x, y, w, c):
    """Conteúdo do painel do núcleo (o cartão é desenhado antes, em build())."""
    eyebrow(x + 24, y + 38, c["core_eyebrow"], "cold")
    text(x + 24, y + 76, c["core_title"], "card")
    text(x + 24, y + 100, c["core_lede"], "card-sub", max_width=w - 48)
    tx, tw, ty = x + 24, w - 48, y + 122
    row_h = 46
    rows = c["crates"]
    th = row_h * len(rows)
    d.add(
        f'<rect x="{num(tx)}" y="{num(ty)}" width="{num(tw)}" height="{th}" rx="16" '
        f'fill="{COLD}" fill-opacity=".045" stroke="{COLD}" stroke-opacity=".2"/>'
    )
    role_x, items_x = tx + 132, tx + 300
    for i, (crate, role, items) in enumerate(rows):
        ry = ty + i * row_h
        if i:
            d.add(f'<path d="M{num(tx + 16)} {num(ry)}H{num(tx + tw - 16)}" stroke="{COLD}" stroke-opacity=".16"/>')
        mid = ry + row_h / 2
        pill_w = measure(crate, "pill") + 22
        d.add(
            f'<rect x="{num(tx + 18)}" y="{num(mid - 13)}" width="{num(pill_w)}" height="26" rx="13" '
            f'fill="{COLD}" fill-opacity=".14" stroke="{COLD}" stroke-opacity=".45"/>'
        )
        if tx + 18 + pill_w > role_x - 12:
            sys.exit(f"O nome {crate!r} é demasiado comprido para a coluna.")
        text(tx + 18 + pill_w / 2, mid + 4.8, crate, "pill", anchor="middle")
        text(role_x, mid + 5.5, role, "crate", max_width=items_x - role_x - 16)
        text(items_x, mid + 5.2, items, "item", max_width=tx + tw - 20 - items_x)
    return ty + th + 24 - y


def site_strip(x, y, w, h, c):
    card(x, y, w, h, "lilac", dashed=True)
    eyebrow(x + 24, y + 36, c["site_eyebrow"], "white")
    text(x + 24, y + 72, c["site_title"], "card")
    text(x + 24, y + 94, c["site_sub"], "card-sub")
    fx = x + 400
    col = (x + w - 24 - fx) / 2
    for i, fact in enumerate(c["site_facts"]):
        cx = fx + (i % 2) * col
        by = y + 52 + (i // 2) * 30
        check(cx, by - 9, LILAC)
        text(cx + 22, by, fact, "item", max_width=col - 30)


# ---------------------------------------------------------------------------
# Composição
# ---------------------------------------------------------------------------


def build(c):
    # Cabeçalho
    eyebrow(M, 58, c["eyebrow"], "violet")
    text(M, 110, c["title"], "title")
    text(M, 144, c["lede"], "lede", max_width=760)
    # Marca à direita, alinhada pelas linhas de base do título e da frase.
    wordmark_w = text(W - M, 110, "SEALER", "wordmark", anchor="end")
    dot(W - M - wordmark_w - 16, 102, "violet")
    text(W - M, 144, "sealer.pt", "domain", anchor="end")

    # Linha 1: telemóvel, relay, telemóvel. A altura vem do conteúdo.
    row_y = 176
    phone_w, relay_w = 260, 300
    gap = (W - 2 * M - 2 * phone_w - relay_w) / 2
    ax = M
    rx = ax + phone_w + gap
    bx = rx + relay_w + gap
    row_h = LAYERS_Y + 3 * LAYER_H + 2 * LAYER_GAP + 18 + 25 + 22
    stack_card(ax, row_y, phone_w, row_h, c["phone"], "violet", "Android", c["android"], "key", c["android_key"], "lilac")
    stack_card(rx, row_y, relay_w, row_h, c["server"], "cold", "Relay", c["relay"], "lock", c["relay_never"], "cold")
    stack_card(bx, row_y, phone_w, row_h, c["phone"], "violet", "iPhone", c["iphone"], "key", c["iphone_key"], "lilac")
    mid = row_y + LAYERS_Y + LAYER_H + LAYER_GAP + LAYER_H / 2  # à altura da camada do meio
    link(ax + phone_w, rx, mid, LILAC, COLD)
    link(rx + relay_w, bx, mid, COLD, LILAC)

    # Ligação: legenda ao centro
    cap_y = row_y + row_h + 40
    cap_w = measure(c["link"], "caption")
    pill_w = cap_w + 12 + 22 + 32
    px = W / 2 - pill_w / 2
    if px < ax + phone_w / 2 + 24:
        sys.exit("A legenda da ligação é demasiado comprida.")
    d.add(
        f'<rect x="{num(px)}" y="{num(cap_y - 17)}" width="{num(pill_w)}" height="34" rx="17" '
        f'fill="{PANEL_DEEP}" stroke="{COLD}" stroke-opacity=".4"/>'
    )
    lock(px + 18, cap_y - 9, COLD_LIGHT)
    text(px + 18 + 12 + 12, cap_y + 5, c["link"], "caption")

    # Linha 2: o núcleo em detalhe. O cartão vai antes do conteúdo, por isso a
    # altura é calculada primeiro e o conteúdo desenhado a seguir.
    core_y = cap_y + 44
    core_h = 122 + 46 * len(c["crates"]) + 24
    card(M, core_y, W - 2 * M, core_h, "cold")
    core_panel(M, core_y, W - 2 * M, c)
    # Depois do painel, para os pontos das pontas ficarem por cima do contorno.
    connector(ax + phone_w / 2, row_y + row_h, core_y)
    connector(bx + phone_w / 2, row_y + row_h, core_y)

    # Linha 3: o site
    site_y, site_h = core_y + core_h + 24, 112
    site_strip(M, site_y, W - 2 * M, site_h, c)

    # Rodapé: legenda
    foot = site_y + site_h + 40
    text(M, foot, c["footer"], "footer")
    lx = W - M - measure(c["legend_dashed"], "footer")
    text(lx, foot, c["legend_dashed"], "footer")
    d.add(
        f'<path d="M{num(lx - 34)} {num(foot - 4.5)}h24" stroke="{COLD}" stroke-opacity=".7" stroke-width="1.4" '
        f'stroke-dasharray="3 5" stroke-linecap="round"/>'
    )
    lx2 = lx - 34 - 28 - measure(c["legend_solid"], "footer")
    text(lx2, foot, c["legend_solid"], "footer")
    sample = d.x_gradient(lx2 - 34, lx2 - 10, LILAC, COLD)
    d.add(f'<path d="M{num(lx2 - 34)} {num(foot - 4.5)}h24" stroke="url(#{sample})" stroke-width="1.6"/>')
    return foot + 28


def static_defs(height):
    def radial(id_, stops, cx="0", cy="0", r="1.2"):
        s = "".join(f'<stop offset="{o}" stop-color="{col}" stop-opacity="{a}"/>' for o, col, a in stops)
        return f'<radialGradient id="{id_}" cx="{cx}" cy="{cy}" r="{r}">{s}</radialGradient>'

    return "".join(
        [
            '<linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">'
            '<stop offset="0" stop-color="#080a14"/><stop offset="1" stop-color="#04050a"/></linearGradient>',
            radial("bg-violet", [("0", VIOLET, ".24"), ("1", VIOLET, "0")], cx=".1", cy="0", r=".6"),
            radial("bg-cold", [("0", COLD, ".09"), ("1", COLD, "0")], cx=".5", cy=".33", r=".45"),
            '<pattern id="grid" width="28" height="28" patternUnits="userSpaceOnUse">'
            f'<path d="M28 0H0V28" fill="none" stroke="{COLD}" stroke-opacity=".06"/></pattern>',
            '<linearGradient id="panel" x1=".3" y1="0" x2=".7" y2="1">'
            f'<stop offset="0" stop-color="{PANEL}"/><stop offset="1" stop-color="{PANEL_DEEP}"/></linearGradient>',
            radial("glow-lilac", [("0", LILAC, ".09"), (".6", LILAC, "0")]),
            radial("glow-cold", [("0", COLD, ".1"), (".6", COLD, "0")]),
            radial("dot-violet", [("0", "#b8a2ff", "1"), (".55", VIOLET, "1"), ("1", "#26136a", "1")], cx=".35", cy=".3", r=".75"),
            radial("dot-cold", [("0", "#a9c1ff", "1"), (".55", COLD, "1"), ("1", "#1b2e6b", "1")], cx=".35", cy=".3", r=".75"),
            radial("dot-white", [("0", "#ffffff", "1"), (".55", "#c9cde0", "1"), ("1", "#5b6075", "1")], cx=".35", cy=".3", r=".75"),
            '<filter id="blur" x="-2" y="-2" width="5" height="5"><feGaussianBlur stdDeviation="3.5"/></filter>',
            f'<clipPath id="frame"><rect width="{W}" height="{num(height)}" rx="28"/></clipPath>',
        ]
    )


def render(lang):
    global d
    d = Drawing()
    c = CONTENT[lang]
    height = build(c)
    h = num(height)
    background = (
        f'<g clip-path="url(#frame)">'
        f'<rect width="{W}" height="{h}" fill="url(#bg)"/>'
        f'<rect width="{W}" height="{h}" fill="url(#grid)"/>'
        f'<rect width="{W}" height="{h}" fill="url(#bg-violet)"/>'
        f'<rect width="{W}" height="{h}" fill="url(#bg-cold)"/></g>'
        f'<rect x=".5" y=".5" width="{W - 1}" height="{num(height - 1)}" rx="27.5" fill="none" '
        f'stroke="{LILAC}" stroke-opacity=".14"/>'
    )
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {h}" width="{W}" height="{h}" '
        f'role="img" aria-labelledby="title desc" lang="{lang}">'
        f'<title id="title">{c["svg_title"]}</title>'
        f'<desc id="desc">{c["description"]}</desc>'
        f'<defs>{static_defs(height)}{"".join(d.defs)}</defs>'
        f"{background}{''.join(d.out)}</svg>\n"
    )


def main():
    target = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_DIR
    target.mkdir(parents=True, exist_ok=True)
    for lang in CONTENT:
        svg = render(lang)
        path = target / CONTENT[lang]["file"]
        path.write_text(svg, encoding="utf-8")
        print(f"{path}  {len(svg.encode()) / 1024:.0f} KB")


if __name__ == "__main__":
    main()
