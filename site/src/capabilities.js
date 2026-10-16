/**
 * Decide se vale a pena carregar o logótipo 3D.
 *
 * O 3D é um extra: a página está completa com a imagem estática. Por isso
 * só é carregado quando não há nenhuma razão para poupar o aparelho.
 *
 * Parâmetros de URL (para testes e medições):
 *   ?static   força a imagem estática
 *   ?force3d  aceita WebGL por software (por exemplo, navegadores sem GPU em testes)
 */
export function decide3D(params) {
  if (params.has('static')) return no('forced-static');

  if (matchMedia('(prefers-reduced-motion: reduce)').matches) return no('reduced-motion');

  const connection = navigator.connection;
  if (connection?.saveData) return no('save-data');

  if (typeof navigator.deviceMemory === 'number' && navigator.deviceMemory < 2) return no('low-memory');

  if (!hasHardwareWebGL2(params.has('force3d'))) return no('no-webgl2');

  return { ok: true, reason: 'ok' };
}

/**
 * O Three.js atual exige WebGL 2. Com failIfMajorPerformanceCaveat, um
 * navegador que só tenha WebGL por software (lento e gastador de bateria)
 * responde que não, e a página fica com a imagem estática.
 */
function hasHardwareWebGL2(allowSoftware) {
  try {
    const canvas = document.createElement('canvas');
    const gl = canvas.getContext('webgl2', { failIfMajorPerformanceCaveat: !allowSoftware });
    if (!gl) return false;
    gl.getExtension('WEBGL_lose_context')?.loseContext();
    return true;
  } catch {
    return false;
  }
}

function no(reason) {
  return { ok: false, reason };
}
