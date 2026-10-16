#!/usr/bin/env python3
"""
Prepara o logótipo da SEALER para a landing page.

Entrada: uma imagem quadrada do logótipo (PNG ou JPG). Pode ter fundo branco
liso ou fundo já transparente.

Saídas:
  src/assets/logo/logo-512.webp, logo-768.webp, logo-1024.webp
      Logótipo recortado, com transparência. Usado como imagem estática
      (fallback) e como textura da face da frente e do verso do objeto 3D.
  src/assets/logo/logo-96.webp
      Ícone pequeno da barra de topo.
  src/assets/logo/silhouette.json
      Contorno do logótipo, usado para extrudir as paredes laterais em 3D.
  public/favicon-32.png, public/favicon-192.png, public/apple-touch-icon.png

Uso:
  python3 scripts/prepare_logo.py caminho/para/logo.jpg

Dependências: Pillow, numpy, scipy, scikit-image
  pip install pillow numpy scipy scikit-image
"""

import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage as ndi
from skimage import measure

ROOT = Path(__file__).resolve().parent.parent
OUT_ASSETS = ROOT / "src" / "assets" / "logo"
OUT_PUBLIC = ROOT / "public"

# Um pixel conta como fundo se for quase branco e estiver ligado à margem
# da imagem. Os brancos dentro do logótipo (reflexos do vidro) ficam.
WHITE_TOLERANCE = 8
# Diferença de cor a partir da qual um pixel da margem é totalmente opaco.
EDGE_FULL_OPACITY = 90
# As paredes laterais 3D ficam ligeiramente para dentro do contorno, para
# não espreitarem por fora da imagem da frente.
WALL_INSET_PX = 2.5
# Tolerância da simplificação do contorno, em pixels da imagem.
CONTOUR_TOLERANCE_PX = 0.5
# Cor de fundo dos ícones que não suportam transparência (iOS).
ICON_BACKGROUND = (5, 6, 13)


def cut_out(rgb: np.ndarray) -> np.ndarray:
    """Devolve uma imagem RGBA (float 0..255) com o fundo branco removido."""
    rgbf = rgb.astype(np.float32)
    distance_from_white = 255.0 - rgbf.min(axis=2)

    near_white = distance_from_white <= WHITE_TOLERANCE
    labels, _ = ndi.label(near_white)
    border_labels = np.unique(
        np.concatenate([labels[0], labels[-1], labels[:, 0], labels[:, -1]])
    )
    border_labels = border_labels[border_labels != 0]
    background = np.isin(labels, border_labels)
    obj = ndi.binary_fill_holes(~background)
    # Fica só a maior forma: elimina pontos soltos de ruído JPEG no fundo.
    obj_labels, count = ndi.label(obj)
    if count > 1:
        sizes = ndi.sum(obj, obj_labels, range(1, count + 1))
        obj = obj_labels == (int(np.argmax(sizes)) + 1)

    alpha = obj.astype(np.float32)
    # Anel de 1 px na margem: opacidade proporcional à cor, para um recorte
    # suave em vez de serrilhado.
    edge = obj & ~ndi.binary_erosion(obj)
    alpha[edge] = np.clip(distance_from_white[edge] / EDGE_FULL_OPACITY, 0.2, 1.0)

    # Remove a mistura com o branco nos pixels semitransparentes.
    a = alpha[..., None]
    safe_a = np.maximum(a, 1e-3)
    unmixed = (rgbf - (1.0 - a) * 255.0) / safe_a
    color = np.where(a < 1.0, np.clip(unmixed, 0, 255), rgbf)
    return np.dstack([color, alpha * 255.0])


def silhouette(alpha: np.ndarray, size: int) -> dict:
    """Contorno simplificado, normalizado à largura da imagem.

    Coordenadas: x para a direita, y para cima, origem no centro da imagem,
    1.0 = largura total da imagem. A mesma escala é usada para o plano 3D.
    """
    mask = alpha > 127
    mask = ndi.binary_fill_holes(mask)
    # Insere o contorno para dentro de forma uniforme.
    dist_inside = ndi.distance_transform_edt(mask)
    inset = dist_inside > WALL_INSET_PX

    contours = measure.find_contours(inset.astype(np.float32), 0.5)
    main = max(contours, key=len)
    simplified = measure.approximate_polygon(main, tolerance=CONTOUR_TOLERANCE_PX)
    if np.allclose(simplified[0], simplified[-1]):
        simplified = simplified[:-1]

    pts = []
    for row, col in simplified:
        x = (col + 0.5) / size - 0.5
        y = 0.5 - (row + 0.5) / size
        pts.append([round(float(x), 5), round(float(y), 5)])

    # Garante orientação anti-horária (exigida pelo Three.js para o Shape).
    area = 0.0
    for i in range(len(pts)):
        x1, y1 = pts[i]
        x2, y2 = pts[(i + 1) % len(pts)]
        area += x1 * y2 - x2 * y1
    if area < 0:
        pts.reverse()

    ys, xs = np.where(mask)
    bbox = {
        "minX": round(float(xs.min() / size - 0.5), 5),
        "maxX": round(float((xs.max() + 1) / size - 0.5), 5),
        "minY": round(float(0.5 - (ys.max() + 1) / size), 5),
        "maxY": round(float(0.5 - ys.min() / size), 5),
    }
    return {"imageSize": size, "bbox": bbox, "points": pts}


def main() -> None:
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(1)

    src = Image.open(sys.argv[1])
    if src.width != src.height:
        sys.exit("A imagem tem de ser quadrada (por exemplo 1024 x 1024).")

    if src.mode in ("RGBA", "LA") and np.asarray(src.convert("RGBA"))[..., 3].min() < 250:
        rgba = np.asarray(src.convert("RGBA")).astype(np.float32)
        print("Imagem com transparência: o recorte foi mantido.")
    else:
        rgba = cut_out(np.asarray(src.convert("RGB")))
        print("Fundo branco removido.")

    master = Image.fromarray(np.round(rgba).astype(np.uint8), "RGBA")
    if master.width != 1024:
        master = master.resize((1024, 1024), Image.LANCZOS)

    OUT_ASSETS.mkdir(parents=True, exist_ok=True)
    OUT_PUBLIC.mkdir(parents=True, exist_ok=True)

    for size in (512, 768, 1024):
        img = master if size == 1024 else master.resize((size, size), Image.LANCZOS)
        path = OUT_ASSETS / f"logo-{size}.webp"
        img.save(path, "WEBP", quality=88, method=6, alpha_quality=100)
        print(f"{path.relative_to(ROOT)}  {path.stat().st_size / 1024:.0f} KB")

    # Ícone pequeno para a barra de topo (aparece a 32 px; 96 px cobre ecrãs 3x).
    icon_path = OUT_ASSETS / "logo-96.webp"
    master.crop(master.getbbox()).resize((96, 96), Image.LANCZOS).save(icon_path, "WEBP", quality=90, method=6)
    print(f"{icon_path.relative_to(ROOT)}  {icon_path.stat().st_size / 1024:.1f} KB")

    data = silhouette(np.asarray(master)[..., 3], master.width)
    (OUT_ASSETS / "silhouette.json").write_text(json.dumps(data, separators=(",", ":")))
    print(f"silhouette.json  {len(data['points'])} pontos")

    # Ícones: recortados ao objeto com uma pequena margem.
    bbox = master.getbbox()
    obj = master.crop(bbox)
    side = max(obj.size)
    square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    square.paste(obj, ((side - obj.width) // 2, (side - obj.height) // 2))

    for name, size, margin, bg in (
        ("favicon-32.png", 32, 0.02, None),
        ("favicon-192.png", 192, 0.06, None),
        ("apple-touch-icon.png", 180, 0.14, ICON_BACKGROUND),
    ):
        inner = round(size * (1 - 2 * margin))
        icon = square.resize((inner, inner), Image.LANCZOS)
        canvas = Image.new("RGBA", (size, size), (*bg, 255) if bg else (0, 0, 0, 0))
        offset = (size - inner) // 2
        canvas.alpha_composite(icon, (offset, offset))
        if bg:
            canvas = canvas.convert("RGB")
        canvas.save(OUT_PUBLIC / name, optimize=True)
        print(f"public/{name}")


if __name__ == "__main__":
    main()
