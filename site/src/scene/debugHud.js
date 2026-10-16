/**
 * Painel de medição, só com ?debug na URL. Serve para medir num telemóvel
 * real: fotogramas por segundo, tempo por fotograma, densidade de píxeis,
 * draw calls e triângulos.
 */
export function createDebugHud(renderer, quality) {
  const element = document.createElement('div');
  element.className = 'debug-hud';
  element.setAttribute('aria-hidden', 'true');
  document.body.append(element);

  let elapsed = 0;

  return {
    update(dt) {
      elapsed += dt;
      if (elapsed < 0.5) return;
      elapsed = 0;
      const { calls, triangles } = renderer.info.render;
      const { stats } = quality;
      element.textContent = [
        `fps        ${stats.fps || '...'}`,
        `ms/frame   ${stats.frameMs || '...'}`,
        `pixelRatio ${stats.pixelRatio}`,
        `draw calls ${calls}`,
        `triângulos ${triangles}`,
        `texturas   ${renderer.info.memory.textures}`,
      ].join('\n');
    },
    dispose() {
      element.remove();
    },
  };
}
