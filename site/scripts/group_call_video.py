#!/usr/bin/env python3
"""
Animação das chamadas de grupo para o site (Como funciona, secção das chamadas).

O ecrã da chamada de grupo das apps, desenhado de novo (sem capturas): a minha câmara no quadrado
grande em cima e os outros 4 numa grelha de 2 por baixo, com as cores da app (o gradiente da barra
de topo, a moldura de quem fala, o vermelho de desligar), o logótipo e o nome "Sealer" na letra
das apps (Manrope Bold). As pessoas são silhuetas, nunca caras. Sem texto nenhum além do nome da
marca, para servir nas 10 línguas.

Em 12 s, em ciclo: sozinho na chamada (os três pontos do logótipo a pulsar), entram 4 pessoas uma
a uma, falam à vez (moldura na cor da app), uma liga a câmara, os botões escondem-se e voltam
(como na app), e saem todas para o ciclo recomeçar.

Saídas (em public/):
  grupo.mp4, grupo.jpg            da esquerda para a direita
  grupo-rtl.mp4, grupo-rtl.jpg    da direita para a esquerda (árabe): marca à direita, a grelha
                                  e os botões espelhados, o logótipo nunca espelhado

Uso:
  python3 scripts/group_call_video.py                 # os dois vídeos e as duas imagens
  python3 scripts/group_call_video.py --frame 200 --png /tmp/x.png [--rtl]   # uma imagem só

Dependências: Pillow, numpy, ffmpeg (com libx264) no PATH.
"""

import argparse
import math
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parent.parent
REPO = ROOT.parent
LOGO = ROOT / "src" / "assets" / "logo" / "logo-512.webp"
FONT = REPO / "branding" / "fontes" / "Manrope-Bold.ttf"
OUT = ROOT / "public"

SIZE = 1080          # vídeo quadrado, como o federacao.mp4
SS = 2               # desenho a 2x e redução (cantos e círculos suaves)
FPS = 30
SECONDS = 12
FRAMES = FPS * SECONDS
POSTER_FRAME = 7 * FPS + 10   # todos na chamada, alguém a falar, sem botões

# Cores da app (android/.../Theme.kt e CallScreen.kt).
TOPBAR = [(0x1E, 0x10, 0x52), (0x43, 0x21, 0xB0), (0x70, 0x38, 0xF0)]
RED = (0xD6, 0x45, 0x45)
AVATAR = (0x7E, 0x57, 0xC2)
BG = (7, 6, 15)


def s(v):
    """Coordenadas do desenho (1080) para a tela grande (x SS)."""
    return int(round(v * SS))


def ease(t):
    t = min(max(t, 0.0), 1.0)
    return 1 - (1 - t) ** 3


def ramp(t, a, b):
    """0 antes de a, 1 depois de b, suave pelo meio."""
    if b <= a:
        return 1.0 if t >= a else 0.0
    return ease((t - a) / (b - a))


def lerp(c1, c2, f):
    return tuple(int(round(a + (b - a) * f)) for a, b in zip(c1, c2))


def vgradient(w, h, top, bottom):
    f = np.linspace(0.0, 1.0, h)[:, None]
    arr = np.array(top, float)[None, None, :] * (1 - f[..., None]) + np.array(bottom, float)[None, None, :] * f[..., None]
    arr = np.repeat(arr, w, axis=1)
    return Image.fromarray(arr.astype(np.uint8), "RGB").convert("RGBA")


def dgradient(w, h, c1, c2):
    """Gradiente na diagonal (canto de cima à esquerda para o de baixo à direita)."""
    x = np.linspace(0, 1, w)[None, :]
    y = np.linspace(0, 1, h)[:, None]
    f = np.clip((x + y) / 2, 0, 1)[..., None]
    arr = np.array(c1, float) * (1 - f) + np.array(c2, float) * f
    return Image.fromarray(arr.astype(np.uint8), "RGB").convert("RGBA")


def rounded_mask(w, h, r):
    m = Image.new("L", (w, h), 0)
    ImageDraw.Draw(m).rounded_rectangle((0, 0, w - 1, h - 1), r, fill=255)
    return m


# ---------------------------------------------------------------------------- ícones (brancos)

def icon_mic(d, cx, cy, size, color=(255, 255, 255, 255)):
    w, h = size * 0.36, size * 0.56
    d.rounded_rectangle((cx - w / 2, cy - size * 0.5, cx + w / 2, cy - size * 0.5 + h), w / 2, fill=color)
    lw = max(2, int(size * 0.08))
    r = size * 0.32
    d.arc((cx - r, cy - size * 0.5 + h * 0.2, cx + r, cy - size * 0.5 + h * 0.2 + 2 * r), 0, 180, fill=color, width=lw)
    d.line((cx, cy - size * 0.5 + h * 0.2 + 2 * r, cx, cy + size * 0.5), fill=color, width=lw)


def icon_camera(d, cx, cy, size, color=(255, 255, 255, 255)):
    bw, bh = size * 0.62, size * 0.5
    x0 = cx - size * 0.5
    d.rounded_rectangle((x0, cy - bh / 2, x0 + bw, cy + bh / 2), size * 0.1, fill=color)
    d.polygon([(x0 + bw + size * 0.04, cy - size * 0.04), (cx + size * 0.5, cy - bh / 2), (cx + size * 0.5, cy + bh / 2), (x0 + bw + size * 0.04, cy + size * 0.04)], fill=color)


def icon_speaker(d, cx, cy, size, color=(255, 255, 255, 255)):
    x0 = cx - size * 0.45
    d.rectangle((x0, cy - size * 0.14, x0 + size * 0.18, cy + size * 0.14), fill=color)
    d.polygon([(x0 + size * 0.18, cy - size * 0.14), (x0 + size * 0.42, cy - size * 0.36), (x0 + size * 0.42, cy + size * 0.36), (x0 + size * 0.18, cy + size * 0.14)], fill=color)
    lw = max(2, int(size * 0.08))
    for r in (size * 0.22, size * 0.4):
        d.arc((x0 + size * 0.42 - r, cy - r, x0 + size * 0.42 + r, cy + r), -50, 50, fill=color, width=lw)


def icon_switch(d, cx, cy, size, color=(255, 255, 255, 255)):
    bw, bh = size * 0.86, size * 0.62
    d.rounded_rectangle((cx - bw / 2, cy - bh / 2 + size * 0.06, cx + bw / 2, cy + bh / 2 + size * 0.06), size * 0.12, outline=color, width=max(2, int(size * 0.08)))
    d.rectangle((cx - size * 0.16, cy - bh / 2 - size * 0.04, cx + size * 0.16, cy - bh / 2 + size * 0.08), fill=color)
    lw = max(2, int(size * 0.07))
    r = size * 0.16
    d.arc((cx - r, cy + size * 0.06 - r, cx + r, cy + size * 0.06 + r), 200, 520, fill=color, width=lw)
    tip = (cx + r * math.cos(math.radians(200)), cy + size * 0.06 + r * math.sin(math.radians(200)))
    d.polygon([(tip[0] - size * 0.08, tip[1] - size * 0.02), (tip[0] + size * 0.05, tip[1] - size * 0.06), (tip[0] + size * 0.0, tip[1] + size * 0.08)], fill=color)


def icon_hangup(d, cx, cy, size, color=(255, 255, 255, 255)):
    lw = max(3, int(size * 0.2))
    r = size * 0.42
    d.arc((cx - r, cy - r * 0.55, cx + r, cy + r * 1.45), 200, 340, fill=color, width=lw)
    for side in (-1, 1):
        x = cx + side * r * 0.86
        d.rounded_rectangle((x - size * 0.13, cy - size * 0.02, x + size * 0.13, cy + size * 0.2), size * 0.05, fill=color)


def person_glyph(d, cx, cy, size, color):
    d.ellipse((cx - size * 0.2, cy - size * 0.42, cx + size * 0.2, cy - size * 0.02), fill=color)
    d.pieslice((cx - size * 0.38, cy + size * 0.04, cx + size * 0.38, cy + size * 0.8), 180, 360, fill=color)


# ---------------------------------------------------------------------------- pessoas

# Fundo (topo, base) e cor da silhueta de cada participante. "hair": forma do cabelo.
PEOPLE = [
    {"bg": ((0x1A, 0x1A, 0x44), (0x4A, 0x52, 0xA8)), "fg": (0x12, 0x10, 0x2C), "hair": "short", "phase": 0.0, "window": 0.64},   # eu
    {"bg": ((0x3A, 0x22, 0x46), (0x9C, 0x5A, 0x7C)), "fg": (0x24, 0x12, 0x2A), "hair": "long", "phase": 1.3, "window": 0.08},
    {"bg": ((0x10, 0x34, 0x40), (0x2C, 0x86, 0x82)), "fg": (0x0A, 0x22, 0x2A), "hair": "bun", "phase": 2.1, "window": 0.66},
    {"bg": ((0x36, 0x26, 0x1A), (0xB0, 0x7C, 0x48)), "fg": (0x24, 0x18, 0x10), "hair": "curly", "phase": 0.7, "window": 0.1},
    {"bg": ((0x1C, 0x2A, 0x48), (0x5C, 0x7E, 0xC4)), "fg": (0x10, 0x18, 0x30), "hair": "short", "phase": 2.8, "window": 0.6},
]


def draw_person(img, p, t, speaking):
    """Uma silhueta (cabeça e ombros) a mexer-se um pouco, como numa imagem de câmara."""
    w, h = img.size
    d = ImageDraw.Draw(img)
    amp = 0.012 if not speaking else 0.02
    dx = w * amp * math.sin(2 * math.pi * t / 3.4 + p["phase"])
    dy = h * (0.006 if not speaking else 0.012) * math.sin(2 * math.pi * t / (1.1 if speaking else 4.2) + p["phase"])
    cx, hy = w / 2 + dx, h * 0.44 + dy
    hr = h * 0.155
    fg = p["fg"] + (235,)
    # Ombros e tronco.
    d.ellipse((cx - w * 0.36, h * 0.64 + dy * 0.5, cx + w * 0.36, h * 1.32), fill=fg)
    d.rounded_rectangle((cx - w * 0.08, hy + hr * 0.6, cx + w * 0.08, h * 0.74), w * 0.04, fill=fg)
    # Cabelo por trás da cabeça.
    if p["hair"] == "long":
        d.rounded_rectangle((cx - hr * 1.25, hy - hr * 1.1, cx + hr * 1.25, hy + hr * 1.7), hr, fill=fg)
    elif p["hair"] == "bun":
        d.ellipse((cx - hr * 0.5, hy - hr * 1.75, cx + hr * 0.5, hy - hr * 0.75), fill=fg)
    elif p["hair"] == "curly":
        for k in range(7):
            a = math.pi * (0.95 + k * 0.18)
            ox, oy = cx + math.cos(a) * hr * 0.95, hy + math.sin(a) * hr * 0.95
            d.ellipse((ox - hr * 0.42, oy - hr * 0.42, ox + hr * 0.42, oy + hr * 0.42), fill=fg)
    d.ellipse((cx - hr, hy - hr, cx + hr, hy + hr), fill=fg)


def camera_tile(w, h, p, t, speaking):
    img = vgradient(w, h, *p["bg"])
    # Uma "janela" desfocada ao fundo, para parecer uma divisão.
    glow = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    wx = p.get("window", 0.62)
    gd.rounded_rectangle((w * wx, h * 0.08, w * (wx + 0.28), h * 0.4), w * 0.03, fill=(255, 255, 255, 26))
    img.alpha_composite(glow.filter(ImageFilter.GaussianBlur(s(6))))
    draw_person(img, p, t, speaking)
    return img


def avatar_tile(w, h):
    img = vgradient(w, h, TOPBAR[0], TOPBAR[1])
    d = ImageDraw.Draw(img)
    r = min(w, h) * 0.22
    cx, cy = w / 2, h * 0.44
    d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=AVATAR + (255,))
    person_glyph(d, cx, cy + r * 0.12, r * 1.3, (255, 255, 255, 235))
    return img


def finish_tile(content, speaking, camera, sp_f):
    """Etiqueta em baixo (só o ícone), cantos redondos e a moldura (cor da app em quem fala)."""
    w, h = content.size
    d = ImageDraw.Draw(content)
    lab = vgradient(w, s(46), (0, 0, 0), (0, 0, 0))
    a = np.linspace(0, 150, lab.size[1]).astype(np.uint8)
    lab.putalpha(Image.fromarray(np.repeat(a[:, None], w, axis=1), "L"))
    content.alpha_composite(lab, (0, h - lab.size[1]))
    (icon_camera if camera else icon_mic)(d, w / 2, h - s(22), s(20))
    radius = s(26)
    mask = rounded_mask(w, h, radius)
    out = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    out.paste(content, (0, 0), mask)
    # Moldura: fio claro; a falar, o gradiente da app por cima (com transição).
    fd = ImageDraw.Draw(out)
    fd.rounded_rectangle((0, 0, w - 1, h - 1), radius, outline=(255, 255, 255, 46), width=s(1.5))
    if sp_f > 0:
        ring = dgradient(w, h, TOPBAR[1], TOPBAR[2])
        rm = Image.new("L", (w, h), 0)
        ImageDraw.Draw(rm).rounded_rectangle((0, 0, w - 1, h - 1), radius, outline=int(255 * sp_f), width=s(5))
        ring.putalpha(rm)
        out.alpha_composite(ring)
    return out


# ---------------------------------------------------------------------------- linha do tempo

JOIN = [None, 0.8, 1.6, 2.4, 3.2]          # quando entra cada um (eu ja estou)
LEAVE = [None, 11.45, 11.3, 11.15, 11.0]   # e quando sai (pela ordem inversa)
SPEAK = [(3.9, 5.5, 1), (5.7, 7.2, 0), (7.4, 8.9, 3), (9.1, 10.5, 2), (10.6, 11.0, 4)]
CAM_ON_4 = 6.4                              # o 4.º liga a câmara
CONTROLS_HIDE = (3.5, 4.0)
CONTROLS_SHOW = (10.5, 10.9)


def speaking_level(i, t):
    lvl = 0.0
    for a, b, who in SPEAK:
        if who == i:
            lvl = max(lvl, min(ramp(t, a, a + 0.25), 1 - ramp(t, b, b + 0.25)))
    return lvl


def layout(rtl):
    """Retângulos (x, y, w, h) no desenho de 1080: o meu quadrado e as 4 posições da grelha."""
    me = (375, 140, 330, 330)
    tw, th, gap = 292, 264, 18
    x0 = (SIZE - (2 * tw + gap)) // 2
    cells = []
    for k in range(4):
        row, col = divmod(k, 2)
        if rtl:
            col = 1 - col
        cells.append((x0 + col * (tw + gap), 140 + 330 + gap + row * (th + gap), tw, th))
    return [me] + cells


# ---------------------------------------------------------------------------- fundo e marca

def background(rtl):
    W = s(SIZE)
    img = Image.new("RGBA", (W, W), BG + (255,))
    glow = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    gx = 0.28 if not rtl else 0.72
    gd.ellipse((s(SIZE * (gx - 0.45)), s(-260), s(SIZE * (gx + 0.45)), s(560)), fill=TOPBAR[1] + (70,))
    gd.ellipse((s(SIZE * (1 - gx - 0.4)), s(640), s(SIZE * (1 - gx + 0.4)), s(1400)), fill=TOPBAR[2] + (38,))
    img.alpha_composite(glow.filter(ImageFilter.GaussianBlur(s(120))))
    # Estrelas pequenas, como na barra de topo da app.
    rng = np.random.default_rng(7)
    sd = ImageDraw.Draw(img)
    for _ in range(70):
        x, y = rng.uniform(0, W), rng.uniform(0, W)
        r = rng.uniform(0.6, 1.6) * SS
        sd.ellipse((x - r, y - r, x + r, y + r), fill=(255, 255, 255, int(rng.uniform(30, 110))))
    # Marca: o logótipo (nunca espelhado) e "Sealer" na letra das apps.
    logo = Image.open(LOGO).convert("RGBA").resize((s(84), s(84)), Image.LANCZOS)
    font = ImageFont.truetype(str(FONT), s(46))
    word = "Sealer"
    tw = sd.textlength(word, font=font)
    y = s(40)
    if not rtl:
        img.alpha_composite(logo, (s(52), y))
        sd.text((s(52 + 84 + 14), y + s(42)), word, font=font, fill=(255, 255, 255, 255), anchor="lm")
    else:
        lx = s(SIZE - 52 - 84)
        img.alpha_composite(logo, (lx, y))
        sd.text((lx - s(14) - tw, y + s(42)), word, font=font, fill=(255, 255, 255, 255), anchor="lm")
    return img


def controls(rtl):
    """Os botões da chamada (como na app): microfone, altifalante, câmara, trocar câmara; desligar."""
    W = s(SIZE)
    img = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    btn, gap, pad = 64, 18, 16
    pw = 4 * btn + 3 * gap + 2 * pad
    px, py = (SIZE - pw) / 2, 840
    d.rounded_rectangle((s(px), s(py), s(px + pw), s(py + btn + 2 * pad)), s((btn + 2 * pad) / 2), fill=(0, 0, 0, 170))
    icons = [icon_mic, icon_speaker, icon_camera, icon_switch]
    if rtl:
        icons = icons[::-1]
    for k, ic in enumerate(icons):
        cx = px + pad + btn / 2 + k * (btn + gap)
        cy = py + pad + btn / 2
        on = ic in (icon_speaker,)
        d.ellipse((s(cx - btn / 2), s(cy - btn / 2), s(cx + btn / 2), s(cy + btn / 2)), fill=(255, 255, 255, 64 if on else 38))
        ic(d, s(cx), s(cy), s(26))
    hx, hy, hr = SIZE / 2, py + btn + 2 * pad + 22 + 38, 38
    d.ellipse((s(hx - hr), s(hy - hr), s(hx + hr), s(hy + hr)), fill=RED + (255,))
    icon_hangup(d, s(hx), s(hy), s(36))
    return img


def waiting_dots(t):
    """Os três pontos do logótipo, a pulsar enquanto ninguém entrou."""
    W = s(SIZE)
    img = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    cy = 140 + 330 + 18 + 264 + 9
    for k in range(3):
        pulse = 0.5 + 0.5 * math.sin(2 * math.pi * (t * 1.4 - k * 0.22))
        r = 11 + 4 * pulse
        cx = SIZE / 2 + (k - 1) * 46
        col = lerp(TOPBAR[1], TOPBAR[2], pulse)
        d.ellipse((s(cx - r), s(cy - r), s(cx + r), s(cy + r)), fill=col + (int(150 + 100 * pulse),))
    return img.filter(ImageFilter.GaussianBlur(SS * 0.6))


# ---------------------------------------------------------------------------- uma imagem

class Renderer:
    def __init__(self, rtl):
        self.rtl = rtl
        self.bg = background(rtl)
        self.ctrl = controls(rtl)
        self.rects = layout(rtl)

    def frame(self, n):
        t = n / FPS
        img = self.bg.copy()
        present = [1.0] + [min(ramp(t, JOIN[i], JOIN[i] + 0.45), 1 - ramp(t, LEAVE[i], LEAVE[i] + 0.4)) for i in range(1, 5)]
        nobody = 1 - max(present[1:])
        if nobody > 0.01:
            dots = waiting_dots(t)
            dots.putalpha(Image.fromarray((np.array(dots.getchannel("A"), float) * nobody).astype(np.uint8), "L"))
            img.alpha_composite(dots)
        for i, (x, y, w, h) in enumerate(self.rects):
            a = present[i]
            if a <= 0.01:
                continue
            sp = speaking_level(i, t)
            W, H = s(w), s(h)
            if i == 4:
                cam_f = ramp(t, CAM_ON_4, CAM_ON_4 + 0.5)
                content = avatar_tile(W, H)
                if cam_f > 0:
                    cam = camera_tile(W, H, PEOPLE[i], t, sp > 0.5)
                    content = Image.blend(content, cam, cam_f)
                tile = finish_tile(content, sp > 0.5, cam_f > 0.5, sp)
            else:
                tile = finish_tile(camera_tile(W, H, PEOPLE[i], t, sp > 0.5), sp > 0.5, True, sp)
            scale = 0.92 + 0.08 * a
            if scale < 0.999:
                tile = tile.resize((max(1, int(W * scale)), max(1, int(H * scale))), Image.BILINEAR)
            if a < 0.999:
                tile.putalpha(Image.fromarray((np.array(tile.getchannel("A"), float) * a).astype(np.uint8), "L"))
            ox = s(x) + (W - tile.size[0]) // 2
            oy = s(y) + (H - tile.size[1]) // 2
            img.alpha_composite(tile, (ox, oy))
        show = 1 - ramp(t, *CONTROLS_HIDE) + ramp(t, *CONTROLS_SHOW)
        show = min(max(show, 0.0), 1.0)
        if show > 0.01:
            c = self.ctrl
            if show < 0.999:
                c = c.copy()
                c.putalpha(Image.fromarray((np.array(c.getchannel("A"), float) * show).astype(np.uint8), "L"))
            img.alpha_composite(c)
        return img.convert("RGB").resize((SIZE, SIZE), Image.LANCZOS)


def encode(rtl, out_mp4, out_jpg):
    r = Renderer(rtl)
    cmd = [
        "ffmpeg", "-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{SIZE}x{SIZE}",
        "-r", str(FPS), "-i", "-", "-an", "-c:v", "libx264", "-preset", "veryslow", "-crf", "30",
        "-tune", "animation", "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(out_mp4),
    ]
    p = subprocess.Popen(cmd, stdin=subprocess.PIPE)
    for n in range(FRAMES):
        f = r.frame(n)
        p.stdin.write(f.tobytes())
        if n == POSTER_FRAME:
            f.resize((800, 800), Image.LANCZOS).save(out_jpg, quality=84, optimize=True, progressive=True)
        if n % 60 == 0:
            print(f"  {'rtl' if rtl else 'ltr'}: {n}/{FRAMES}", file=sys.stderr)
    p.stdin.close()
    if p.wait() != 0:
        sys.exit("ffmpeg falhou")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--frame", type=int)
    ap.add_argument("--png")
    ap.add_argument("--rtl", action="store_true")
    a = ap.parse_args()
    if a.frame is not None:
        Renderer(a.rtl).frame(a.frame).save(a.png or "/tmp/grupo.png")
        return
    encode(False, OUT / "grupo.mp4", OUT / "grupo.jpg")
    encode(True, OUT / "grupo-rtl.mp4", OUT / "grupo-rtl.jpg")


if __name__ == "__main__":
    main()
