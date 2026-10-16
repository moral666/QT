/**
 * Ajusta a qualidade ao que o aparelho aguenta.
 *
 * Mede o tempo médio por fotograma em janelas de alguns segundos:
 *  - se estiver lento, baixa a densidade de píxeis um degrau;
 *  - se, já no mínimo, continuar lento em duas janelas seguidas,
 *    pede para desistir do 3D (a página volta à imagem estática).
 *
 * Também acumula estatísticas para o painel ?debug e para os testes.
 */
export function createQualityGovernor({ initialPixelRatio, cfg, minPixelRatio, fixed, onPixelRatio, onGiveUp }) {
  let pixelRatio = initialPixelRatio;
  let windowTime = 0;
  let windowFrames = 0;
  let slowStrikes = 0;
  let cooldown = 1; // ignora o primeiro segundo (compilação de shaders, upload de texturas)

  const stats = {
    frames: 0,
    fps: 0,
    frameMs: 0,
    pixelRatio,
    downgrades: 0,
  };

  function sample(dt) {
    stats.frames += 1;
    if (cooldown > 0) {
      cooldown -= dt;
      return;
    }
    windowTime += dt;
    windowFrames += 1;
    if (windowTime < cfg.sampleWindow) return;

    const frameMs = (windowTime / windowFrames) * 1000;
    stats.frameMs = Math.round(frameMs * 10) / 10;
    stats.fps = Math.round(windowFrames / windowTime);
    windowTime = 0;
    windowFrames = 0;
    if (fixed) return;

    if (frameMs > cfg.slowFrameMs && pixelRatio > minPixelRatio) {
      pixelRatio = Math.max(minPixelRatio, pixelRatio - cfg.pixelRatioStep);
      stats.pixelRatio = pixelRatio;
      stats.downgrades += 1;
      slowStrikes = 0;
      cooldown = 0.5;
      onPixelRatio(pixelRatio);
    } else if (frameMs > cfg.giveUpFrameMs && pixelRatio <= minPixelRatio) {
      slowStrikes += 1;
      if (slowStrikes >= 2) onGiveUp('slow');
    } else {
      slowStrikes = 0;
    }
  }

  /** Depois de uma pausa (separador escondido, fora do ecrã) recomeça a medir do zero. */
  function resetWindow() {
    windowTime = 0;
    windowFrames = 0;
    cooldown = 0.5;
  }

  return {
    sample,
    resetWindow,
    stats,
    get pixelRatio() {
      return pixelRatio;
    },
  };
}
