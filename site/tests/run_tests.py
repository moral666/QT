#!/usr/bin/env python3
"""
Testes de compatibilidade e desempenho da landing page da SEALER.

Uso (na pasta do projeto):
    npm run build
    python3 tests/run_tests.py

Requisitos:
    pip install playwright pillow numpy
    python3 -m playwright install chromium

Resultados: tests/results/results.json e capturas de ecrã em tests/results/.

Notas importantes sobre os números:
  - Estes testes correm num Chromium sem GPU. O WebGL é feito por software
    (SwiftShader), por isso os fps e os tempos da cena 3D são PESSIMISTAS e
    não representam um telemóvel real. Para medir num telemóvel, abra a
    página com ?debug no fim do endereço.
  - O parâmetro ?force3d obriga a página a aceitar WebGL por software (sem
    ele, a página recusa-o e mostra a imagem estática, como faria a um
    visitante sem GPU).
  - A CSP real fica ativa durante os testes. Por isso o script usa
    avaliações diretas (page.evaluate) e não wait_for_function, que depende
    de eval e é bloqueada pela CSP.
"""

import base64
import gzip
import io
import json
import math
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

import numpy as np
from PIL import Image
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "dist"
OUT = ROOT / "tests" / "results"
PORT = 4173
BASE = f"http://localhost:{PORT}/"

SWIFTSHADER = ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"]
SLOW_4G = {"offline": False, "latency": 150, "downloadThroughput": 1.6 * 1024 * 1024 / 8, "uploadThroughput": 750 * 1024 / 8}

# Corre antes de qualquer script da página (não é afetado pela CSP).
INIT_SCRIPT = """
window.__t = { csp: [], lcp: 0, fcp: 0, longTasks: [], has3dAt: null, firstFrame: null };
document.addEventListener('securitypolicyviolation', (e) => window.__t.csp.push(e.violatedDirective + ' ' + e.blockedURI));
try {
  new PerformanceObserver((l) => { for (const e of l.getEntries()) window.__t.lcp = e.startTime; })
    .observe({ type: 'largest-contentful-paint', buffered: true });
  new PerformanceObserver((l) => { for (const e of l.getEntries()) if (e.name === 'first-contentful-paint') window.__t.fcp = e.startTime; })
    .observe({ type: 'paint', buffered: true });
  new PerformanceObserver((l) => { for (const e of l.getEntries()) window.__t.longTasks.push([Math.round(e.startTime), Math.round(e.duration)]); })
    .observe({ type: 'longtask', buffered: true });
} catch (e) {}
// Regista o instante em que o 3D aparece e copia o primeiro fotograma
// (ainda dentro da mesma tarefa, antes de o navegador limpar o buffer).
new MutationObserver(() => {
  const stage = document.getElementById('stage');
  if (!stage || !stage.classList.contains('has-3d') || window.__t.has3dAt !== null) return;
  window.__t.has3dAt = performance.now();
  const canvas = stage.querySelector('canvas');
  window.__t.initialPixelRatio = canvas ? Math.round((canvas.width / canvas.clientWidth) * 100) / 100 : null;
  if (window.__captureFirstFrame) {
    try {
      window.__t.firstFrame = canvas.toDataURL('image/png');
      // A mesma imagem, desenhada pelo próprio navegador no mesmo sítio e tamanho.
      const img = document.getElementById('logo-img');
      const ref = document.createElement('canvas');
      ref.width = canvas.width;
      ref.height = canvas.height;
      const ctx = ref.getContext('2d');
      ctx.imageSmoothingQuality = 'high';
      const size = canvas.width / 1.5;
      const offset = (canvas.width - size) / 2;
      ctx.drawImage(img, offset, offset, size, size);
      window.__t.reference = ref.toDataURL('image/png');
    } catch (e) { window.__t.firstFrame = 'erro: ' + e.message; }
  }
}).observe(document, { attributes: true, subtree: true, attributeFilter: ['class'] });
"""

results = {"tests": [], "notes": []}


# ---------- utilitários ----------

def record(name, passed, **data):
    results["tests"].append({"name": name, "passed": bool(passed), **data})
    mark = "OK " if passed else "FALHOU"
    print(f"[{mark}] {name}")
    for key, value in data.items():
        if key not in ("requests",):
            print(f"        {key}: {value}")


def poll(page, js, timeout_s=60, interval_ms=100):
    end = time.time() + timeout_s
    while time.time() < end:
        try:
            if page.evaluate(js):
                return True
        except Exception:
            pass
        page.wait_for_timeout(interval_ms)
    return False


def collect_console(page):
    messages = []
    page.on("console", lambda m: messages.append({"type": m.type, "text": m.text}))
    page.on("pageerror", lambda e: messages.append({"type": "pageerror", "text": str(e)}))
    return messages


def problems(messages):
    """Erros e avisos na consola. Ignora o aviso de desempenho causado pela
    própria captura do primeiro fotograma (toDataURL obriga a ler a GPU)."""
    return [
        m for m in messages
        if m["type"] in ("error", "warning", "pageerror") and "ReadPixels" not in m["text"]
    ]


def track_requests(page):
    requests = []

    def on_finished(request):
        try:
            sizes = request.sizes()
            body = sizes.get("responseBodySize", 0)
        except Exception:
            body = 0
        requests.append({"url": request.url.replace(BASE, "/"), "bytes": body})

    page.on("requestfinished", on_finished)
    return requests


def mode(page):
    return page.evaluate("document.documentElement.dataset.logoMode || ''")


def timings(page):
    return page.evaluate(
        """(() => {
          const t = window.__t || {};
          const long = t.longTasks || [];
          return {
            fcp_ms: Math.round(t.fcp || 0),
            lcp_ms: Math.round(t.lcp || 0),
            first_3d_frame_ms: t.has3dAt === null ? null : Math.round(t.has3dAt),
            long_tasks: long.length,
            longest_task_ms: long.reduce((m, x) => Math.max(m, x[1]), 0),
            blocking_ms: long.reduce((s, x) => s + Math.max(0, x[1] - 50), 0),
            csp_violations: t.csp || [],
          };
        })()"""
    )


def wait_for_server():
    for _ in range(100):
        try:
            urllib.request.urlopen(BASE, timeout=1)
            return True
        except Exception:
            time.sleep(0.2)
    return False


def gzip_size(path):
    return len(gzip.compress(path.read_bytes(), compresslevel=9))


# ---------- testes ----------

def test_bundle_sizes():
    assets = DIST / "assets"
    def find(prefix, ext):
        return sorted(assets.glob(f"{prefix}*{ext}"))
    initial = [DIST / "index.html", *find("index-", ".css"), *find("index-", ".js")]
    fonts = find("", ".woff2")
    logo_mobile = find("logo-512", ".webp")
    logo_desktop = find("logo-1024", ".webp")
    extra_3d = [*find("three-", ".js"), *find("logoScene-", ".js")]

    def total(files, compressed=True):
        return sum(gzip_size(f) if compressed and f.suffix in (".html", ".css", ".js") else f.stat().st_size for f in files)

    data = {
        "html_css_js_iniciais_kb_gzip": round(total(initial) / 1024, 1),
        "tipos_de_letra_kb": round(total(fonts, False) / 1024, 1),
        "logo_telemovel_kb": round(total(logo_mobile, False) / 1024, 1),
        "logo_desktop_kb": round(total(logo_desktop, False) / 1024, 1),
        "modulo_3d_kb_gzip": round(total(extra_3d) / 1024, 1),
    }
    first_view_mobile = data["html_css_js_iniciais_kb_gzip"] + data["tipos_de_letra_kb"] + data["logo_telemovel_kb"]
    data["primeira_vista_telemovel_kb"] = round(first_view_mobile, 1)
    data["com_3d_telemovel_kb"] = round(first_view_mobile + data["modulo_3d_kb_gzip"], 1)
    passed = data["html_css_js_iniciais_kb_gzip"] < 15 and first_view_mobile < 200 and data["com_3d_telemovel_kb"] < 400
    record("Peso dos ficheiros (objetivo: JS/CSS/HTML iniciais < 15 KB; 1.a vista < 200 KB; com 3D < 400 KB)", passed, **data)


def test_desktop_3d(browser):
    ctx = browser.new_context(viewport={"width": 1440, "height": 900}, device_scale_factor=1)
    page = ctx.new_page()
    page.add_init_script("window.__captureFirstFrame = true;")
    page.add_init_script(INIT_SCRIPT)
    messages = collect_console(page)
    requests = track_requests(page)
    page.goto(BASE + "?force3d&quality=fixed")
    ok = poll(page, "document.documentElement.dataset.logoMode === '3d'")
    page.wait_for_timeout(300)
    page.screenshot(path=str(OUT / "desktop-3d-inicio.png"))

    # Primeiro fotograma 3D comparado com a imagem estática
    comparison = compare_first_frame(page.evaluate("window.__t.firstFrame"), page.evaluate("window.__t.reference"))

    # Estatísticas depois de alguns segundos
    page.wait_for_timeout(5000)
    stats = page.evaluate(
        """(() => { const s = window.__sealerLogo; const i = s.renderer.info;
          return { fps_software: s.stats.fps, ms_por_fotograma: s.stats.frameMs, pixelRatio: s.stats.pixelRatio,
                   draw_calls: i.render.calls, triangulos: i.render.triangles, texturas: i.memory.textures,
                   programas_shader: i.programs ? i.programs.length : null }; })()"""
    )
    t = timings(page)
    page.screenshot(path=str(OUT / "desktop-3d.png"))

    issues = problems(messages)
    passed = ok and not issues and not t["csp_violations"] and comparison.get("iou", 0) > 0.985 and stats["draw_calls"] <= 6
    record(
        "Desktop 1440x900 com 3D",
        passed,
        modo=mode(page),
        **t,
        **stats,
        primeiro_fotograma_vs_imagem=comparison,
        consola=issues or "sem erros nem avisos",
        pedidos=len(requests),
    )
    ctx.close()


def decode_data_url(data_url):
    return Image.open(io.BytesIO(base64.b64decode(data_url.split(",", 1)[1]))).convert("RGBA")


def compare_first_frame(frame_url, reference_url):
    """Compara o primeiro fotograma 3D com a imagem estática desenhada pelo
    navegador no mesmo sítio. Mede a sobreposição das silhuetas (IoU, 1 =
    iguais) e a diferença média de cor dentro do logótipo (0 a 255)."""
    if not frame_url or not str(frame_url).startswith("data:image/png") or not reference_url:
        return {"erro": str(frame_url)[:80]}
    f = np.asarray(decode_data_url(frame_url)).astype(np.float32)
    e = np.asarray(decode_data_url(reference_url)).astype(np.float32)
    mask_f = f[..., 3] > 127
    mask_e = e[..., 3] > 127
    iou = (mask_f & mask_e).sum() / max(1, (mask_f | mask_e).sum())
    inside = mask_e & mask_f
    diff = np.abs(f[..., :3] - e[..., :3])[inside].mean(axis=1)
    Image.fromarray(np.uint8(np.clip(np.abs(f - e)[..., :3] * 4, 0, 255))).save(OUT / "primeiro-fotograma-diferenca-x4.png")
    return {
        "iou": round(float(iou), 4),
        "diferenca_media_0_255": round(float(diff.mean()), 2),
        "diferenca_p95_0_255": round(float(np.percentile(diff, 95)), 2),
    }


def test_interaction(browser):
    ctx = browser.new_context(viewport={"width": 1280, "height": 860})
    page = ctx.new_page()
    page.add_init_script(INIT_SCRIPT)
    messages = collect_console(page)
    page.goto(BASE + "?force3d&quality=fixed")
    poll(page, "document.documentElement.dataset.logoMode === '3d'")
    TAU = 2 * math.pi
    at_rest = (
        "(() => { const s = window.__sealerLogo.motion.spin; const T = Math.PI * 2;"
        " return Math.abs(s - Math.round(s / T) * T) < 0.02; })()"
    )
    # Espera pelo fim da volta de apresentação (a WebGL por software anda devagar).
    page.wait_for_timeout(1000)
    poll(page, at_rest, timeout_s=20)
    spin0 = page.evaluate("window.__sealerLogo.motion.spin")

    # Arrastar com o rato
    box = page.query_selector("#stage").bounding_box()
    cx, cy = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
    page.mouse.move(cx, cy)
    page.mouse.down()
    for step in range(1, 11):
        page.mouse.move(cx + step * 15, cy, steps=2)
        page.wait_for_timeout(16)
    mid_spin = page.evaluate("window.__sealerLogo.motion.spin") - spin0
    dragging_class = page.evaluate("document.getElementById('stage').classList.contains('is-dragging')")
    page.mouse.up()
    released = time.time()
    back_to_front = poll(page, at_rest, timeout_s=20)
    settle_s = round(time.time() - released, 2)

    # Tocar/clicar: uma volta completa
    before = page.evaluate("window.__sealerLogo.motion.spin")
    page.mouse.click(cx, cy)
    page.wait_for_timeout(500)
    poll(page, at_rest, timeout_s=20)
    after = page.evaluate("window.__sealerLogo.motion.spin")
    full_turn = abs((after - before) - TAU) < 0.05

    # Ao explorar a grelha, o logótipo sai de cena e deixa de ser desenhado; Esc volta ao início
    frames = lambda: page.evaluate("window.__sealerLogo.stats.frames")
    page.mouse.move(60, 60)
    page.mouse.wheel(0, 240)
    page.wait_for_timeout(400)
    frames_a = frames()
    page.wait_for_timeout(1500)
    frames_b = frames()
    page.keyboard.press("Escape")
    poll(page, f"window.__sealerLogo.stats.frames > {frames_b}", timeout_s=4)
    frames_c = frames()

    # Modo lista: com o logótipo fora do ecrã, o desenho para
    page.click("#view-toggle")
    page.wait_for_timeout(500)
    page.evaluate("window.scrollTo(0, document.body.scrollHeight)")
    page.wait_for_timeout(400)
    frames_d = frames()
    page.wait_for_timeout(1500)
    frames_e = frames()
    page.evaluate("window.scrollTo(0, 0)")
    poll(page, f"window.__sealerLogo.stats.frames > {frames_e}", timeout_s=4)
    frames_f = frames()

    # Separador escondido (simulado)
    page.evaluate(
        "Object.defineProperty(document, 'hidden', { configurable: true, get: () => true });"
        "document.dispatchEvent(new Event('visibilitychange'));"
    )
    page.wait_for_timeout(200)
    frames_g = frames()
    page.wait_for_timeout(1200)
    frames_h = frames()

    issues = problems(messages)
    passed = (
        mid_spin > 0.5 and dragging_class and back_to_front and full_turn
        and frames_b - frames_a == 0 and frames_c > frames_b
        and frames_e - frames_d == 0 and frames_f > frames_e
        and frames_h - frames_g == 0 and not issues
    )
    record(
        "Logótipo 3D: arrastar, tocar e poupança de energia (a explorar, fora do ecrã, separador escondido)",
        passed,
        rotacao_a_meio_do_arrasto_rad=round(mid_spin, 2),
        volta_a_frente_depois_de_largar=back_to_front,
        segundos_ate_parar_com_webgl_por_software=settle_s,
        clique_da_uma_volta=full_turn,
        fotogramas_em_1_5s_a_explorar_a_grelha=frames_b - frames_a,
        retoma_ao_voltar_ao_inicio=frames_c > frames_b,
        fotogramas_em_1_5s_fora_do_ecra_na_lista=frames_e - frames_d,
        retoma_ao_voltar_ao_topo=frames_f > frames_e,
        fotogramas_em_1_2s_com_separador_escondido=frames_h - frames_g,
        consola=issues or "sem erros nem avisos",
    )
    ctx.close()


def test_angles(browser):
    """Capturas em ângulos fixos, para comparar com as vistas de rotação de referência."""
    ctx = browser.new_context(viewport={"width": 900, "height": 900})
    page = ctx.new_page()
    page.goto(BASE + "?force3d&quality=fixed")
    poll(page, "document.documentElement.dataset.logoMode === '3d'")
    # Espera pelo fim da volta de apresentação, para os ângulos partirem da frente.
    page.wait_for_timeout(1000)
    poll(
        page,
        "(() => { const s = window.__sealerLogo.motion.spin; const T = Math.PI * 2;"
        " return Math.abs(s - Math.round(s / T) * T) < 0.01; })()",
        timeout_s=20,
    )
    box = page.query_selector("#stage").bounding_box()
    clip = {"x": box["x"] - box["width"] * 0.25, "y": box["y"] - box["height"] * 0.25, "width": box["width"] * 1.5, "height": box["height"] * 1.5}
    page.evaluate("window.__sealerLogo.motion.startDrag()")
    frames = []
    for deg in (0, 30, 60, 90, 120, 180):
        page.evaluate(f"window.__sealerLogo.motion.moveDrag({deg} / 360, 0)")
        page.wait_for_timeout(400)
        frames.append(Image.open(io.BytesIO(page.screenshot(clip=clip))).convert("RGB"))
    page.evaluate("window.__sealerLogo.motion.endDrag()")
    size = 300
    sheet = Image.new("RGB", (size * len(frames), size))
    for i, frame in enumerate(frames):
        sheet.paste(frame.resize((size, size), Image.LANCZOS), (i * size, 0))
    sheet.save(OUT / "angulos-0-30-60-90-120-180.png")
    record("Capturas em ângulos fixos (0, 30, 60, 90, 120 e 180 graus)", True, ficheiro="tests/results/angulos-0-30-60-90-120-180.png")
    ctx.close()


def mobile_context(browser, cpu_rate=4, network=True):
    ctx = browser.new_context(viewport={"width": 390, "height": 844}, device_scale_factor=3, is_mobile=True, has_touch=True)
    page = ctx.new_page()
    cdp = ctx.new_cdp_session(page)
    cdp.send("Emulation.setCPUThrottlingRate", {"rate": cpu_rate})
    if network:
        cdp.send("Network.enable")
        cdp.send("Network.emulateNetworkConditions", SLOW_4G)
    return ctx, page, cdp


def test_mobile(browser, force3d):
    ctx, page, cdp = mobile_context(browser)
    page.add_init_script(INIT_SCRIPT)
    messages = collect_console(page)
    requests = track_requests(page)
    page.goto(BASE + ("?force3d" if force3d else ""))
    page.wait_for_load_state("load")
    if force3d:
        ok = poll(page, "document.documentElement.dataset.logoMode === '3d'", timeout_s=90)
        # Tempo para o regulador de qualidade medir: num aparelho lento deve
        # baixar a resolução e, se não chegar, voltar suavemente à imagem.
        page.wait_for_timeout(12000)
        stats = page.evaluate(
            """({ pixelRatio_inicial: window.__t.initialPixelRatio,
                  pixelRatio_final: window.__sealerLogo ? window.__sealerLogo.stats.pixelRatio : null,
                  descidas_de_resolucao: window.__sealerLogo ? window.__sealerLogo.stats.downgrades : null,
                  fps_software: window.__sealerLogo ? window.__sealerLogo.stats.fps : null })"""
        )
    else:
        ok = poll(page, "(document.documentElement.dataset.logoMode || '').startsWith('static')", timeout_s=30)
        page.wait_for_timeout(1500)
        stats = {}
    t = timings(page)
    three_loaded = any("/three-" in r["url"] for r in requests)
    total_kb = round(sum(r["bytes"] for r in requests) / 1024, 1)
    name = "mobile-3d" if force3d else "mobile-sem-gpu"
    page.screenshot(path=str(OUT / f"{name}.png"))
    issues = problems(messages)
    if force3d:
        final_mode = mode(page)
        graceful = final_mode in ("3d", "static:slow")
        passed = ok and graceful and t["lcp_ms"] < 2500 and stats.get("pixelRatio_inicial") == 1.5 and not issues
        title = "Telemóvel (390x844, ecrã 3x, CPU 4x mais lento, rede 4G lenta) com 3D por software"
    else:
        passed = ok and t["lcp_ms"] < 2500 and not three_loaded and mode(page) == "static:no-webgl2" and not issues
        title = "Telemóvel sem GPU (WebGL só por software: a página recusa o 3D), CPU 4x, rede 4G lenta"
    record(title, passed, modo=mode(page), **t, **stats, three_js_descarregado=three_loaded, bytes_transferidos_kb=total_kb, consola=issues or "sem erros nem avisos")
    ctx.close()


def test_energy(browser):
    """Tempo de CPU do fio principal por segundo, em repouso: estática vs 3D."""
    out = {}
    for label, query in (("estatica", "?static"), ("3d", "?force3d&quality=fixed")):
        ctx = browser.new_context(viewport={"width": 1280, "height": 860})
        page = ctx.new_page()
        cdp = ctx.new_cdp_session(page)
        cdp.send("Performance.enable")
        page.goto(BASE + query)
        if label == "3d":
            poll(page, "document.documentElement.dataset.logoMode === '3d'")
        page.wait_for_timeout(4000)

        def metrics():
            return {m["name"]: m["value"] for m in cdp.send("Performance.getMetrics")["metrics"]}

        a = metrics()
        page.wait_for_timeout(5000)
        b = metrics()
        seconds = b["Timestamp"] - a["Timestamp"]
        out[label] = {
            "cpu_fio_principal_ms_por_s": round((b["TaskDuration"] - a["TaskDuration"]) * 1000 / seconds, 1),
            "memoria_js_mb": round(b["JSHeapUsedSize"] / 1024 / 1024, 1),
        }
        ctx.close()
    passed = out["estatica"]["cpu_fio_principal_ms_por_s"] < 20
    record("Consumo em repouso (CPU do fio principal por segundo de animação)", passed, **out)


def test_static_paths(browser):
    # 1. Movimento reduzido
    ctx = browser.new_context(viewport={"width": 1280, "height": 860}, reduced_motion="reduce")
    page = ctx.new_page()
    messages = collect_console(page)
    requests = track_requests(page)
    page.goto(BASE + "?force3d")
    page.wait_for_timeout(2500)
    animations = page.evaluate("document.getElementById('logo-img').getAnimations().length")
    three = any("/three-" in r["url"] for r in requests)
    record(
        "Movimento reduzido (prefers-reduced-motion): sem 3D e sem animação",
        mode(page) == "static:reduced-motion" and animations == 0 and not three and not problems(messages),
        modo=mode(page), animacoes_ativas=animations, three_js_descarregado=three,
    )
    ctx.close()

    # 2. ?static
    ctx = browser.new_context(viewport={"width": 1280, "height": 860})
    page = ctx.new_page()
    requests = track_requests(page)
    page.goto(BASE + "?static")
    page.wait_for_timeout(1500)
    animations = page.evaluate("document.getElementById('logo-img').getAnimations().length")
    three = any("/three-" in r["url"] for r in requests)
    record("Versão estática forçada (?static): imagem a flutuar em CSS", mode(page) == "static:forced-static" and animations == 1 and not three, modo=mode(page), animacoes_ativas=animations, three_js_descarregado=three)
    ctx.close()


def test_failures(playwright):
    # 1. Navegador sem WebGL nenhum
    browser = playwright.chromium.launch(args=["--disable-3d-apis"])
    page = browser.new_page()
    messages = collect_console(page)
    page.goto(BASE + "?force3d")
    page.wait_for_timeout(2000)
    record("Navegador sem WebGL: imagem estática", mode(page) == "static:no-webgl2" and not problems(messages), modo=mode(page))
    browser.close()

    browser = playwright.chromium.launch(args=SWIFTSHADER)
    # 2. A imagem do logótipo não carrega
    page = browser.new_page()
    messages = collect_console(page)
    page.route("**/logo-*.webp", lambda route: route.abort())
    page.goto(BASE + "?force3d")
    page.wait_for_timeout(2500)
    missing = page.evaluate("document.getElementById('stage').classList.contains('is-logo-missing')")
    page.screenshot(path=str(OUT / "falha-logotipo.png"))
    errors = [m for m in messages if m["type"] == "pageerror"]
    record("A imagem do logótipo não carrega: balão em CSS, página utilizável", missing and mode(page).startswith("static") and not errors, modo=mode(page), balao_css=missing, erros_de_script=errors or "nenhum")
    page.close()

    # 3. O módulo 3D (Three.js) não carrega
    page = browser.new_page()
    messages = collect_console(page)
    page.route("**/three-*.js", lambda route: route.abort())
    page.goto(BASE + "?force3d")
    page.wait_for_timeout(3000)
    visible = page.evaluate("getComputedStyle(document.getElementById('logo-img')).opacity === '1'")
    errors = [m for m in messages if m["type"] == "pageerror"]
    record("O Three.js não carrega: fica a imagem estática", mode(page) == "static:error" and visible and not errors, modo=mode(page), imagem_visivel=visible, erros_de_script=errors or "nenhum")
    page.close()
    browser.close()


def test_responsive(browser):
    sizes = [(320, 568), (360, 740), (390, 844), (768, 1024), (1024, 768), (1440, 900), (1920, 1080), (2560, 1440)]
    report = {}
    ok = True
    for w, h in sizes:
        ctx = browser.new_context(viewport={"width": w, "height": h}, reduced_motion="reduce")
        page = ctx.new_page()
        page.goto(BASE)
        poll(page, "window.__sealerLayout.field.state.cells > 0", timeout_s=10)
        page.wait_for_timeout(200)
        grid = page.evaluate(
            """(() => ({
              excesso: document.documentElement.scrollWidth - innerWidth,
              janela: window.__sealerLayout.field.state.cellSize,
              cortadas: [...document.querySelectorAll('#field-layer .tile:not(.is-clone)')]
                .filter((t) => t.scrollHeight > t.clientHeight + 1).map((t) => (t.querySelector('h3') || {}).textContent),
            }))()"""
        )
        page.screenshot(path=str(OUT / f"grelha-{w}x{h}.png"))
        page.goto(BASE + "#lista")
        page.wait_for_timeout(400)
        over_list = page.evaluate("document.documentElement.scrollWidth - innerWidth")
        page.screenshot(path=str(OUT / f"lista-{w}x{h}.png"))
        report[f"{w}x{h}"] = f"janela {grid['janela']} px, excesso grelha {grid['excesso']}, excesso lista {over_list}, texto cortado {len(grid['cortadas'])}"
        ok = ok and grid["excesso"] <= 0 and over_list <= 0 and not grid["cortadas"]
        ctx.close()
    record("8 tamanhos de ecrã (320 a 2560 px): sem deslocamento horizontal e o texto cabe em todas as janelas", ok, **report)


def test_accessibility(browser):
    ctx = browser.new_context(viewport={"width": 1280, "height": 860}, reduced_motion="reduce")
    page = ctx.new_page()
    page.goto(BASE)
    poll(page, "window.__sealerLayout.field.state.cells > 0", timeout_s=10)
    data = page.evaluate(
        """(() => ({
          lang: document.documentElement.lang,
          h1: document.querySelectorAll('h1').length,
          alt: document.getElementById('logo-img').alt.length > 10,
          janelas_focaveis: document.querySelectorAll('#field-layer .tile[tabindex="0"]').length,
          janelas_com_titulo: [...document.querySelectorAll('#field-layer article.tile:not(.is-clone)')].every((t) => t.getAttribute('aria-labelledby') && document.getElementById(t.getAttribute('aria-labelledby'))),
          ids_repetidos: (() => { const ids = [...document.querySelectorAll('[id]')].map((e) => e.id); return ids.length - new Set(ids).size; })(),
        }))()"""
    )
    # Tab: a primeira janela focada vem para o centro
    page.focus("#view-toggle")
    for _ in range(3):
        page.keyboard.press("Tab")
    page.wait_for_timeout(300)
    focused_tile = page.evaluate("document.activeElement.closest('#field-layer .tile') !== null")
    exploring = page.evaluate("document.documentElement.classList.contains('is-exploring')")
    # O botão da lista funciona com o teclado
    page.focus("#view-toggle")
    page.keyboard.press("Enter")
    page.wait_for_timeout(300)
    list_mode = page.evaluate("document.documentElement.classList.contains('mode-list')")
    list_headings = page.evaluate("[...document.querySelectorAll('h1, h2')].map((h) => h.textContent.trim()).join(' | ')")
    passed = (
        data["lang"] == "pt-PT" and data["h1"] == 1 and data["alt"] and data["janelas_focaveis"] == 22
        and data["janelas_com_titulo"] and data["ids_repetidos"] == 0 and focused_tile and exploring and list_mode
    )
    record(
        "Acessibilidade (idioma, um h1, texto alternativo, janelas focáveis com Tab, lista pelo teclado, ids únicos)",
        passed,
        **data,
        tab_leva_a_uma_janela=focused_tile,
        tab_entra_no_modo_explorar=exploring,
        lista_abre_com_enter=list_mode,
        titulos_da_lista=list_headings,
    )
    ctx.close()


def test_grid(browser):
    """A grelha de janelas: arrastar, encaixar, tocar, setas, roda, Esc e lista."""
    ctx = browser.new_context(viewport={"width": 1440, "height": 900})
    page = ctx.new_page()
    messages = collect_console(page)
    page.goto(BASE + "?static")
    poll(page, "window.__sealerLayout.field.state.cells > 0", timeout_s=10)
    page.wait_for_timeout(300)
    state = lambda: page.evaluate("window.__sealerLayout.field.state")
    settled = "!window.__sealerLayout.field.state.settling && !window.__sealerLayout.field.state.dragging"
    s0 = state()
    pitch = s0["pitch"]

    # Arrastar com o rato: o início sai de cena e a grelha encaixa com uma janela ao centro
    page.mouse.move(200, 700)
    page.mouse.down()
    for i in range(1, 21):
        page.mouse.move(200 + i * 18, 700 - i * 7)
        page.wait_for_timeout(16)
    pressed = page.evaluate("document.getElementById('field').classList.contains('is-pressed')")
    page.mouse.up()
    exploring = page.evaluate("document.documentElement.classList.contains('is-exploring')")
    hero_inert = page.evaluate("document.getElementById('hero').inert")
    poll(page, settled, timeout_s=10)
    s1 = state()
    snapped = abs(s1["ox"] / pitch - round(s1["ox"] / pitch)) < 0.01 and abs(s1["oy"] / pitch - round(s1["oy"] / pitch)) < 0.01
    moved = abs(s1["ox"] - s0["ox"]) > pitch * 0.5

    # Tocar numa janela que não está ao centro: vem para o centro
    target = page.evaluate(
        """(() => { const tiles = [...document.querySelectorAll('#field-layer article.tile')];
          const c = { x: innerWidth / 2, y: innerHeight / 2 };
          const t = tiles.map((el) => ({ el, r: el.getBoundingClientRect() }))
            .filter(({ r }) => r.left > 40 && r.right < innerWidth - 40 && r.top > 60 && r.bottom < innerHeight - 40)
            .map(({ el, r }) => ({ id: el.getAttribute('aria-labelledby'), x: r.left + r.width / 2, y: r.top + r.height / 2, d: Math.hypot(r.left + r.width / 2 - c.x, r.top + r.height / 2 - c.y) }))
            .sort((a, b) => b.d - a.d)[0];
          return t; })()"""
    )
    page.mouse.click(target["x"], target["y"])
    # O ponteiro ao centro anula o desvio que a grelha faz a seguir o rato.
    page.mouse.move(720, 450)
    poll(page, settled, timeout_s=10)
    page.wait_for_timeout(1200)
    centred = page.evaluate(
        f"""(() => {{ const r = document.querySelector('[aria-labelledby="{target['id']}"]').getBoundingClientRect();
          return Math.hypot(r.left + r.width / 2 - innerWidth / 2, r.top + r.height / 2 - innerHeight / 2); }})()"""
    )

    # Setas: uma janela de cada vez
    before = state()
    page.keyboard.press("ArrowRight")
    poll(page, settled, timeout_s=10)
    after = state()
    arrow_ok = abs((before["ox"] - after["ox"]) - pitch) < 1 and abs(before["oy"] - after["oy"]) < 1

    # Roda do rato
    page.mouse.move(300, 300)
    page.mouse.wheel(0, pitch * 1.2)
    page.wait_for_timeout(500)
    poll(page, settled, timeout_s=10)
    after_wheel = state()
    wheel_ok = abs(after_wheel["oy"] - after["oy"]) >= pitch - 1 and abs(after_wheel["oy"] / pitch - round(after_wheel["oy"] / pitch)) < 0.01

    # Esc: volta ao início (logótipo ao centro, entre as quatro janelas centrais)
    page.keyboard.press("Escape")
    poll(page, settled, timeout_s=10)
    home = state()
    home_ok = (
        not page.evaluate("document.documentElement.classList.contains('is-exploring')")
        and abs(((home["ox"] / pitch) % 1) - 0.5) < 0.01 and abs(((home["oy"] / pitch) % 1) - 0.5) < 0.01
    )

    # Lista e volta: as janelas regressam aos seus lugares
    page.click("#view-toggle")
    page.wait_for_timeout(300)
    in_list = page.evaluate("document.querySelectorAll('.group__tiles .tile').length")
    in_grid_while_list = page.evaluate("document.querySelectorAll('#field-layer .tile').length")
    hash_list = page.evaluate("location.hash")
    page.click("#view-toggle")
    page.wait_for_timeout(300)
    back = page.evaluate("document.querySelectorAll('#field-layer .tile').length")

    issues = problems(messages)
    passed = (
        pressed and exploring and hero_inert and moved and snapped and centred < 2 and arrow_ok and wheel_ok and home_ok
        and in_list == 22 and in_grid_while_list == 0 and hash_list == "#lista" and back == s0["cells"] and not issues
    )
    record(
        "Grelha de janelas (arrastar, recuo ao pressionar, encaixe, tocar, setas, roda, Esc, lista)",
        passed,
        janela_px=s0["cellSize"],
        recua_ao_pressionar=pressed,
        entra_no_modo_explorar=exploring,
        inicio_fica_inerte=hero_inert,
        encaixa_numa_janela=snapped,
        janela_tocada_fica_ao_centro_px=round(centred, 1),
        seta_avanca_uma_janela=arrow_ok,
        roda_move_e_encaixa=wheel_ok,
        esc_volta_ao_inicio=home_ok,
        janelas_na_lista=in_list,
        endereco_da_lista=hash_list,
        janelas_ao_voltar_a_grelha=back,
        consola=issues or "sem erros nem avisos",
    )
    ctx.close()


def test_grid_performance(browser):
    """Custo do arrasto: o movimento deve mexer só em transformações (sem layout)."""
    results_by_device = {}
    for label, viewport, scale, mobile, cpu in (
        ("desktop", {"width": 1440, "height": 900}, 1, False, 1),
        ("telemovel_cpu_4x", {"width": 390, "height": 844}, 3, True, 4),
    ):
        ctx = browser.new_context(viewport=viewport, device_scale_factor=scale, is_mobile=mobile, has_touch=mobile)
        page = ctx.new_page()
        cdp = ctx.new_cdp_session(page)
        cdp.send("Emulation.setCPUThrottlingRate", {"rate": cpu})
        cdp.send("Performance.enable")
        page.goto(BASE + "?static")
        page.wait_for_timeout(1500)
        page.evaluate(
            """window.__frames = []; (function loop(t) { if (!window.__frames) return; window.__frames.push(t); requestAnimationFrame(loop); })(performance.now());"""
        )

        def metrics():
            return {m["name"]: m["value"] for m in cdp.send("Performance.getMetrics")["metrics"]}

        a = metrics()
        x, y = viewport["width"] * 0.3, viewport["height"] * 0.6
        page.mouse.move(x, y)
        page.mouse.down()
        steps = 90
        for i in range(steps):
            # vaivém de 2 a 3 segundos, como quem explora
            dx = 6 if (i // 30) % 2 == 0 else -6
            x += dx
            y += 2
            page.mouse.move(x, y)
            page.wait_for_timeout(16)
        page.mouse.up()
        b = metrics()
        frames = page.evaluate(
            """(() => { const f = window.__frames; const d = []; for (let i = 1; i < f.length; i++) d.push(f[i] - f[i - 1]);
              d.sort((p, q) => p - q); return { n: d.length, p50: d[Math.floor(d.length * 0.5)], p95: d[Math.floor(d.length * 0.95)] }; })()"""
        )
        seconds = b["Timestamp"] - a["Timestamp"]
        results_by_device[label] = {
            "recalculos_de_layout": int(b["LayoutCount"] - a["LayoutCount"]),
            "cpu_fio_principal_ms_por_s": round((b["TaskDuration"] - a["TaskDuration"]) * 1000 / seconds, 1),
            "fotograma_mediano_ms": round(frames["p50"], 1),
            "fotograma_p95_ms": round(frames["p95"], 1),
        }

        # Em repouso, a grelha não gasta nada
        page.evaluate("window.__frames = null")
        poll(page, "!window.__sealerLayout.field.state.settling", timeout_s=10)
        page.wait_for_timeout(1000)
        c = metrics()
        page.wait_for_timeout(3000)
        d = metrics()
        results_by_device[label]["cpu_em_repouso_ms_por_s"] = round((d["TaskDuration"] - c["TaskDuration"]) * 1000 / (d["Timestamp"] - c["Timestamp"]), 1)
        ctx.close()

    passed = all(r["recalculos_de_layout"] <= 6 and r["cpu_em_repouso_ms_por_s"] < 20 for r in results_by_device.values())
    record("Desempenho do arrasto da grelha (layout, CPU e fotogramas; repouso)", passed, **results_by_device)


def main():
    if not (DIST / "index.html").exists():
        sys.exit("Falta a pasta dist/. Corra primeiro: npm run build")
    OUT.mkdir(parents=True, exist_ok=True)
    server = subprocess.Popen(
        ["npx", "vite", "preview", "--port", str(PORT), "--strictPort"],
        cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    try:
        if not wait_for_server():
            sys.exit("O servidor de pré-visualização não arrancou.")
        test_bundle_sizes()
        with sync_playwright() as p:
            browser = p.chromium.launch(args=SWIFTSHADER)
            results["browser"] = f"Chromium {browser.version} (WebGL por software)"
            test_desktop_3d(browser)
            test_interaction(browser)
            test_angles(browser)
            test_mobile(browser, force3d=True)
            test_energy(browser)
            test_static_paths(browser)
            browser.close()
            # Sem SwiftShader: composição normal, e o navegador só tem WebGL por
            # software, que a página recusa (fica a imagem estática).
            plain = p.chromium.launch()
            test_grid(plain)
            test_grid_performance(plain)
            test_responsive(plain)
            test_accessibility(plain)
            test_mobile(plain, force3d=False)
            plain.close()
            test_failures(p)
    finally:
        server.terminate()

    passed = sum(t["passed"] for t in results["tests"])
    results["summary"] = f"{passed} de {len(results['tests'])} testes passaram"
    (OUT / "results.json").write_text(json.dumps(results, indent=2, ensure_ascii=False))
    print("\n" + results["summary"])
    sys.exit(0 if passed == len(results["tests"]) else 1)


if __name__ == "__main__":
    main()
