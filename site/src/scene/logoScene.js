/**
 * Logótipo da SEALER em 3D.
 *
 * O objeto é feito de três partes:
 *  - face da frente: um plano com a imagem do logótipo, igual pixel a pixel
 *    à imagem estática quando está de frente;
 *  - verso: o mesmo plano visto por trás, por isso o logótipo aparece
 *    espelhado, como nas vistas de rotação fornecidas;
 *  - paredes laterais: uma faixa lilás iridescente extrudida a partir do
 *    contorno do logótipo, com a espessura medida na vista de perfil.
 *
 * Usa a mesma imagem que já está na página como textura: não há segundo
 * download. Este módulo só é carregado depois do conteúdo principal.
 */
import {
  AdditiveBlending,
  CanvasTexture,
  DirectionalLight,
  DoubleSide,
  Group,
  MathUtils,
  Mesh,
  MeshBasicMaterial,
  MeshPhysicalMaterial,
  MeshStandardMaterial,
  NoToneMapping,
  PerspectiveCamera,
  PlaneGeometry,
  PMREMGenerator,
  Scene,
  SRGBColorSpace,
  Texture,
  WebGLRenderer,
} from 'three';
import { RoomEnvironment } from 'three/addons/environments/RoomEnvironment.js';

import silhouette from '../assets/logo/silhouette.json';
import { LOOK, MOTION, QUALITY, RENDER, SCENE } from './config.js';
import { createDebugHud } from './debugHud.js';
import { createWallGeometry } from './geometry.js';
import { attachInput } from './input.js';
import { createMotion } from './motion.js';
import { createQualityGovernor } from './quality.js';

/** Duração do cruzamento entre o 3D e a imagem, igual à transição em styles.css. */
const CROSSFADE_MS = 450;

/**
 * @param {object} options
 * @param {HTMLElement} options.stage Elemento .stage
 * @param {HTMLImageElement} options.image Imagem do logótipo já carregada
 * @param {boolean} [options.debug] Mostra o painel de medição
 * @param {boolean} [options.allowSoftwareRendering] Aceita WebGL por software (só para testes)
 * @param {boolean} [options.fixedQuality] Desliga a qualidade adaptativa (só para testes)
 * @param {(reason: string) => void} options.onFallback Chamado se o 3D tiver de ser desligado depois de montado
 * @returns {Promise<{ dispose: () => void, stats: object }>}
 */
export async function mountLogoScene({ stage, image, debug, allowSoftwareRendering, fixedQuality, onFallback }) {
  // Antes de criar o canvas: se falhar, não fica nada a meio na página.
  const textureSource = await pixelExactCopy(image);

  const canvas = document.createElement('canvas');
  canvas.className = 'stage__canvas';
  canvas.setAttribute('aria-hidden', 'true');
  stage.append(canvas);

  let renderer;
  try {
    renderer = new WebGLRenderer({
      canvas,
      alpha: true,
      antialias: true,
      powerPreference: RENDER.powerPreference,
      failIfMajorPerformanceCaveat: !allowSoftwareRendering,
    });
  } catch (error) {
    canvas.remove();
    throw error;
  }

  try {
    return await buildScene({ stage, canvas, renderer, textureSource, debug, fixedQuality, onFallback });
  } catch (error) {
    // Qualquer falha a meio da montagem: liberta a GPU e devolve a página à imagem.
    renderer.dispose();
    renderer.forceContextLoss();
    canvas.remove();
    throw error;
  }
}

async function buildScene({ stage, canvas, renderer, textureSource, debug, fixedQuality, onFallback }) {
  renderer.setClearColor(0x000000, 0);
  renderer.outputColorSpace = SRGBColorSpace;
  // Sem tone mapping: a face da frente tem de reproduzir as cores da imagem.
  renderer.toneMapping = NoToneMapping;

  const isTouch = matchMedia('(pointer: coarse)').matches;
  const maxPixelRatio = isTouch ? RENDER.maxPixelRatioMobile : RENDER.maxPixelRatioDesktop;
  const initialPixelRatio = Math.max(RENDER.minPixelRatio, Math.min(devicePixelRatio || 1, maxPixelRatio));

  // ----- Cena, câmara e luz -----
  const scene = new Scene();

  const roomEnvironment = new RoomEnvironment();
  const pmrem = new PMREMGenerator(renderer);
  // Mapa de reflexos pequeno (128 px por face): os reflexos são suaves e
  // a geração custa um quarto da do tamanho padrão (256 px).
  const environment = pmrem.fromScene(roomEnvironment, 0.04, 0.1, 100, { size: 128 });
  scene.environment = environment.texture;
  pmrem.dispose();
  roomEnvironment.dispose?.();

  const faceZ = SCENE.depth / 2 + 0.0005;
  const camera = new PerspectiveCamera(SCENE.fov, 1, 0.1, 20);
  // Distância a que um plano em faceZ ocupa exatamente SCENE.frame unidades de altura:
  // assim a face da frente tem o mesmo tamanho que a imagem estática.
  camera.position.set(0, 0, faceZ + SCENE.frame / 2 / Math.tan(MathUtils.degToRad(SCENE.fov / 2)));

  for (const light of [LOOK.keyLight, LOOK.rimLight]) {
    const directional = new DirectionalLight(light.color, light.intensity);
    directional.position.set(...light.position);
    scene.add(directional);
  }

  // ----- Objeto -----
  const floatGroup = new Group();
  const tiltGroup = new Group();
  const spinGroup = new Group();
  floatGroup.add(tiltGroup);
  tiltGroup.add(spinGroup);
  scene.add(floatGroup);

  const texture = new Texture(textureSource);
  texture.colorSpace = SRGBColorSpace;
  texture.anisotropy = Math.min(4, renderer.capabilities.getMaxAnisotropy());
  texture.needsUpdate = true;

  // A cor difusa é preta e a imagem entra como emissão: de frente, a face
  // mostra a imagem tal como é. Os reflexos do ambiente somam-se por cima
  // e mexem-se quando o objeto roda, o que lhe dá o ar de vidro.
  //
  // As duas faces partilham o mesmo material de dois lados (um só shader
  // para compilar). Visto por trás, o plano do verso mostra a imagem
  // espelhada, como nas vistas de rotação de referência.
  const faceMaterial = new MeshStandardMaterial({
    color: 0x000000,
    map: texture,
    emissive: 0xffffff,
    emissiveMap: texture,
    metalness: 0,
    roughness: LOOK.face.roughness,
    envMapIntensity: LOOK.face.envMapIntensity,
    transparent: true,
    premultipliedAlpha: true,
    alphaTest: 0.004,
    side: DoubleSide,
    // Sem isto, o Three.js desenharia cada face transparente em duas passagens.
    forceSinglePass: true,
  });
  const faceGeometry = new PlaneGeometry(1, 1);

  const front = new Mesh(faceGeometry, faceMaterial);
  front.position.z = faceZ;
  const back = new Mesh(faceGeometry, faceMaterial);
  back.position.z = -faceZ;

  const wallGeometry = createWallGeometry(silhouette.points, {
    depth: SCENE.depth,
    bulge: SCENE.wallBulge,
    segments: SCENE.wallSegments,
    normalBend: SCENE.wallNormalBend,
  });
  const wallMaterial = new MeshPhysicalMaterial({ ...LOOK.wall, metalness: 0 });
  const walls = new Mesh(wallGeometry, wallMaterial);

  // As paredes são opacas e desenham-se primeiro; as faces, com transparência, depois.
  walls.renderOrder = 0;
  front.renderOrder = 1;
  back.renderOrder = 1;
  spinGroup.add(walls, front, back);

  // ----- Poça de luz por baixo -----
  const poolTexture = createPoolTexture(LOOK.pool.color);
  const poolMaterial = new MeshBasicMaterial({
    map: poolTexture,
    transparent: true,
    opacity: 0,
    blending: AdditiveBlending,
    depthWrite: false,
  });
  const poolGeometry = new PlaneGeometry(1, 1);
  const pool = new Mesh(poolGeometry, poolMaterial);
  pool.position.set(0, silhouette.bbox.minY - LOOK.pool.gap, 0);
  pool.scale.set(LOOK.pool.width, LOOK.pool.height, 1);
  pool.renderOrder = -1;
  scene.add(pool);

  // Compila os shaders antes do primeiro fotograma. Com a extensão
  // KHR_parallel_shader_compile (presente na maioria dos navegadores), a
  // compilação corre em paralelo e não bloqueia a página. Sem ela, os
  // shaders compilam no primeiro fotograma, como é normal.
  if (renderer.extensions.has('KHR_parallel_shader_compile')) {
    await renderer.compileAsync(scene, camera);
  }

  // ----- Movimento, entrada e qualidade -----
  const motion = createMotion(MOTION);
  const detachInput = attachInput(stage, motion, MOTION);

  let disposed = false;
  const quality = createQualityGovernor({
    initialPixelRatio,
    minPixelRatio: RENDER.minPixelRatio,
    cfg: QUALITY,
    fixed: fixedQuality,
    onPixelRatio: (ratio) => {
      renderer.setPixelRatio(ratio);
      resize();
    },
    onGiveUp: (reason) => fallBack(reason),
  });
  renderer.setPixelRatio(initialPixelRatio);
  const hud = debug ? createDebugHud(renderer, quality) : null;

  function resize() {
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    if (!width || !height) return;
    renderer.setSize(width, height, false);
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
  }
  resize();
  const resizeObserver = new ResizeObserver(resize);
  resizeObserver.observe(canvas);

  // ----- Ciclo de desenho: só corre com o separador visível e o palco no ecrã -----
  let frameId = 0;
  let lastTime = 0;
  let running = false;
  let inView = true;
  let firstFrame = true;
  let leaving = false;
  let active = true;

  function renderFrame(now) {
    frameId = requestAnimationFrame(renderFrame);
    // O movimento usa um passo limitado (sem saltos depois de uma pausa);
    // a medição de qualidade usa o tempo real entre fotogramas.
    const realDt = lastTime ? (now - lastTime) / 1000 : 0;
    const dt = Math.min(realDt, 1 / 15);
    lastTime = now;

    const pose = motion.update(dt);
    floatGroup.position.y = pose.y;
    tiltGroup.rotation.set(pose.pitch, pose.yaw, 0);
    spinGroup.rotation.y = pose.spin;

    // Igual à animação CSS da versão estática: mais fraca e estreita quando o logótipo sobe.
    poolMaterial.opacity = LOOK.pool.opacity * (1 - 0.35 * pose.floatWave);
    pool.scale.x = LOOK.pool.width * (1 - 0.12 * pose.floatWave);

    renderer.render(scene, camera);

    if (firstFrame) {
      firstFrame = false;
      // O fotograma que acabou de ser desenhado coincide com a imagem: troca sem salto.
      stage.classList.add('has-3d');
      setTimeout(() => !disposed && motion.spinOnce(MOTION.introDuration), MOTION.introDelay * 1000);
    }

    if (realDt > 0) quality.sample(realDt);
    hud?.update(dt);
  }

  function start() {
    if (running || disposed) return;
    running = true;
    lastTime = 0;
    quality.resetWindow();
    frameId = requestAnimationFrame(renderFrame);
  }

  function stop() {
    running = false;
    cancelAnimationFrame(frameId);
  }

  function updateRunning() {
    if (!document.hidden && inView && active && !disposed) start();
    else stop();
  }

  /** Liga ou desliga o desenho (por exemplo, enquanto se explora a grelha). */
  function setActive(value) {
    active = value;
    updateRunning();
  }

  const intersectionObserver = new IntersectionObserver((entries) => {
    inView = entries[entries.length - 1].isIntersecting;
    updateRunning();
  });
  intersectionObserver.observe(stage);
  document.addEventListener('visibilitychange', updateRunning);

  // Se a pessoa ativar "reduzir movimento" com a página aberta, desliga o 3D.
  const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');
  const onReducedMotion = () => reducedMotion.matches && fallBack('reduced-motion');
  reducedMotion.addEventListener('change', onReducedMotion);

  const onContextLost = (event) => {
    event.preventDefault();
    fallBack('context-lost');
  };
  canvas.addEventListener('webglcontextlost', onContextLost);

  function dispose() {
    if (disposed) return;
    disposed = true;
    stop();
    detachInput();
    intersectionObserver.disconnect();
    resizeObserver.disconnect();
    document.removeEventListener('visibilitychange', updateRunning);
    reducedMotion.removeEventListener('change', onReducedMotion);
    canvas.removeEventListener('webglcontextlost', onContextLost);
    stage.classList.remove('has-3d', 'is-leaving-3d', 'is-dragging');
    hud?.dispose();
    for (const disposable of [
      faceGeometry,
      wallGeometry,
      poolGeometry,
      faceMaterial,
      wallMaterial,
      poolMaterial,
      texture,
      poolTexture,
      environment,
    ]) {
      disposable.dispose();
    }
    renderer.dispose();
    canvas.remove();
  }

  /** Desliga o 3D com um cruzamento suave para a imagem estática. */
  function fallBack(reason) {
    if (disposed || leaving) return;
    leaving = true;
    stage.classList.add('is-leaving-3d');
    stage.classList.remove('has-3d');
    onFallback?.(reason);
    setTimeout(dispose, CROSSFADE_MS);
  }

  updateRunning();

  // motion fica exposto para os testes poderem fixar ângulos (startDrag/moveDrag).
  return { dispose, setActive, stats: quality.stats, renderer, motion };
}

/**
 * Cópia da imagem do logótipo com as dimensões reais do ficheiro.
 *
 * Numa <img> com srcset, naturalWidth devolve a largura corrigida pela
 * densidade (por exemplo 440 em vez de 512), e o Three.js reservaria uma
 * textura do tamanho errado. Uma Image nova, sem srcset, aponta para o
 * mesmo ficheiro e vem da cache do navegador: não há segundo download.
 */
async function pixelExactCopy(image) {
  const copy = new Image();
  copy.decoding = 'async';
  copy.src = image.currentSrc || image.src;
  await copy.decode();
  return copy;
}

/** Gradiente radial lilás desenhado num canvas pequeno, usado como luz difusa. */
function createPoolTexture(color) {
  const size = 128;
  const canvas = document.createElement('canvas');
  canvas.width = size;
  canvas.height = size;
  const context = canvas.getContext('2d');
  const gradient = context.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
  gradient.addColorStop(0, color);
  gradient.addColorStop(0.45, hexWithAlpha(color, 0.35));
  gradient.addColorStop(1, hexWithAlpha(color, 0));
  context.fillStyle = gradient;
  context.fillRect(0, 0, size, size);
  const texture = new CanvasTexture(canvas);
  texture.colorSpace = SRGBColorSpace;
  return texture;
}

function hexWithAlpha(hex, alpha) {
  const value = parseInt(hex.slice(1), 16);
  return `rgba(${(value >> 16) & 255}, ${(value >> 8) & 255}, ${value & 255}, ${alpha})`;
}
