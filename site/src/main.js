/**
 * Ponto de entrada da página.
 *
 * Ordem de prioridades:
 *  1. O conteúdo (logótipo, nome, frase e janelas) aparece sem esperar por JavaScript pesado.
 *  2. A grelha de janelas é montada logo (src/grid/, sem bibliotecas).
 *  3. Se a imagem do logótipo falhar, mostra-se um balão desenhado em CSS.
 *  4. Depois de a página carregar, e só se o aparelho o justificar, carrega-se
 *     o logótipo 3D (Three.js) num ficheiro à parte.
 */
import './styles.css';
import { decide3D } from './capabilities.js';
import { initLayout } from './layout.js';
import { initStatus } from './status.js';

const root = document.documentElement;
const stage = document.getElementById('stage');
const logo = document.getElementById('logo-img');
const params = new URLSearchParams(location.search);

// ----- Logótipo: ligado só quando está à vista -----

let logoScene = null;
let logoActive = true;

function setLogoActive(active) {
  logoActive = active;
  logoScene?.setActive(active);
}

// ----- Modos da página (grelha, explorar, lista) -----

try {
  window.__sealerLayout = initLayout({ setLogoActive });
} catch (error) {
  // Se a grelha falhar por algum motivo, a página fica em modo lista, que funciona sempre.
  console.info(`[SEALER] Grelha desativada (${error.message}). A mostrar a lista.`);
  root.classList.remove('mode-grid', 'is-exploring');
  root.classList.add('mode-list');
}

// ----- Estado do serviço (canto superior esquerdo) -----

initStatus(document.getElementById('service-status'));

// ----- Logótipo 3D -----

function markLogoMissing() {
  stage.classList.add('is-logo-missing');
}

function useStaticLogo(reason) {
  root.classList.add('is-static');
  root.dataset.logoMode = `static:${reason}`;
}

if (logo.complete && logo.naturalWidth === 0) markLogoMissing();
logo.addEventListener('error', markLogoMissing, { once: true });

const decision = decide3D(params);

if (!decision.ok) {
  useStaticLogo(decision.reason);
} else {
  root.dataset.logoMode = 'loading';
  afterPageLoad(async () => {
    try {
      await imageReady(logo);
      const { mountLogoScene } = await import('./scene/logoScene.js');
      const scene = await mountLogoScene({
        stage,
        image: logo,
        debug: params.has('debug'),
        allowSoftwareRendering: params.has('force3d'),
        fixedQuality: params.get('quality') === 'fixed',
        onFallback: useStaticLogo,
      });
      logoScene = scene;
      scene.setActive(logoActive);
      root.dataset.logoMode = '3d';
      // Usado pelos testes automáticos e útil na consola do navegador.
      window.__sealerLogo = scene;
    } catch (error) {
      console.info(`[SEALER] Logótipo 3D desativado (${error.message}). A mostrar a imagem estática.`);
      useStaticLogo('error');
    }
  });
}

/** Espera pelo evento load e por um momento livre do navegador. */
function afterPageLoad(task) {
  const schedule = () => {
    if ('requestIdleCallback' in window) requestIdleCallback(task, { timeout: 1500 });
    else setTimeout(task, 200);
  };
  if (document.readyState === 'complete') schedule();
  else window.addEventListener('load', schedule, { once: true });
}

function imageReady(image) {
  if (image.complete) {
    return image.naturalWidth > 0 ? image.decode().catch(() => {}) : Promise.reject(new Error('imagem do logótipo indisponível'));
  }
  return new Promise((resolve, reject) => {
    image.addEventListener('load', resolve, { once: true });
    image.addEventListener('error', () => reject(new Error('imagem do logótipo indisponível')), { once: true });
  });
}
