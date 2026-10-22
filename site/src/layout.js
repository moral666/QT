/**
 * Modos da página.
 *
 *   grelha + início    o logótipo, o nome e a frase ao centro, sobre a grelha escurecida;
 *   grelha + explorar  ao primeiro arrasto, roda, tecla ou toque, o logótipo sai de cena e
 *                      a grelha fica em primeiro plano; o nome passa para o canto;
 *   lista              as mesmas janelas em texto corrido, com scroll normal (#lista).
 *
 * As classes vão para <html>: mode-grid ou mode-list, e is-exploring.
 * O modo inicial é escolhido por um script mínimo no <head> do index.html,
 * antes de a página aparecer.
 */
import { createField } from './grid/field.js';
import { L, translatePage } from './i18n.js';

export function initLayout({ setLogoActive }) {
  const root = document.documentElement;
  const hero = document.getElementById('hero');
  const toggle = document.getElementById('view-toggle');
  const brand = document.getElementById('bar-brand');
  const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');

  let mode = root.classList.contains('mode-list') ? 'list' : 'grid';
  let exploring = false;

  const field = createField({
    field: document.getElementById('field'),
    layer: document.getElementById('field-layer'),
    reducedMotion: () => reducedMotion.matches,
    onInteract: () => setExploring(true),
  });

  function setExploring(value) {
    if (mode !== 'grid' || exploring === value) return;
    exploring = value;
    root.classList.toggle('is-exploring', value);
    hero.inert = value;
    setLogoActive(!value);
    if (!value) field.goHome();
  }

  function setMode(next, { updateHash = true } = {}) {
    mode = next;
    exploring = false;
    root.classList.remove('is-exploring');
    root.classList.toggle('mode-grid', next === 'grid');
    root.classList.toggle('mode-list', next === 'list');
    hero.inert = false;
    toggle.textContent = next === 'grid' ? L({ pt: 'Ver em lista', en: 'View as list', fr: 'Voir en liste', es: 'Ver en lista', de: 'Als Liste anzeigen', it: 'Vedi come elenco', ja: 'リストで表示', zh: '列表视图', hi: 'लिस्ट में देखें' }) : L({ pt: 'Ver em grelha', en: 'View as grid', fr: 'Voir en grille', es: 'Ver en cuadrícula', de: 'Als Raster anzeigen', it: 'Vedi come griglia', ja: 'グリッドで表示', zh: '网格视图', hi: 'ग्रिड में देखें' });
    toggle.setAttribute('aria-pressed', String(next === 'list'));

    if (next === 'grid') attachWhenReady();
    else field.detach();

    setLogoActive(true);
    window.scrollTo(0, 0);
    if (updateHash) {
      const url = next === 'list' ? '#lista' : location.pathname + location.search;
      history.replaceState(null, '', url);
    }
  }

  /**
   * Na primeira visita, a grelha só é montada depois de o logótipo estar
   * desenhado: o logótipo é o conteúdo principal e não deve esperar por ela.
   * A grelha aparece a seguir, com um desvanecimento (CSS: .field.is-ready).
   */
  let firstAttach = true;
  function attachWhenReady() {
    const attach = () => {
      if (mode !== 'grid') return;
      field.attach();
      document.getElementById('field').classList.add('is-ready');
    };
    if (!firstAttach) {
      attach();
      return;
    }
    firstAttach = false;
    const logo = document.getElementById('logo-img');
    const painted = logo.complete ? Promise.resolve() : new Promise((resolve) => {
      logo.addEventListener('load', resolve, { once: true });
      logo.addEventListener('error', resolve, { once: true });
    });
    // Dois pedidos de fotograma: quando o segundo corre, o logótipo já foi pintado.
    // As cópias das janelas (ecrãs grandes) são feitas já com o texto na língua da página.
    Promise.all([painted, translatePage()]).then(() => requestAnimationFrame(() => requestAnimationFrame(attach)));
  }

  toggle.addEventListener('click', () => setMode(mode === 'grid' ? 'list' : 'grid'));

  brand.addEventListener('click', () => {
    if (mode === 'grid') setExploring(false);
    else window.scrollTo({ top: 0, behavior: reducedMotion.matches ? 'auto' : 'smooth' });
  });

  window.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && mode === 'grid' && exploring) setExploring(false);
  });

  window.addEventListener('hashchange', () => {
    setMode(location.hash === '#lista' ? 'list' : 'grid', { updateHash: false });
  });

  setMode(mode, { updateHash: false });

  return {
    field,
    get mode() {
      return mode;
    },
    get exploring() {
      return exploring;
    },
  };
}
