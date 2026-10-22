/**
 * Línguas do site: as mesmas seis da app. Português (o texto das páginas), inglês,
 * francês, espanhol, alemão e italiano.
 *
 * Escolha: `?lang=pt|en|fr|es|de|it` no endereço (o seletor PT · EN · FR · ES · DE · IT
 * põe-no); sem isso, a língua do navegador (uma destas; outra qualquer: inglês). Nada é guardado no
 * navegador (a política de cookies diz que o site não usa armazenamento): a escolha
 * passa de página em página pelos links, com o `?lang=`.
 *
 * As páginas marcam os textos com data-i18n="chave" (e data-i18n-alt, -aria-label,
 * -title, -content para atributos). Nas outras línguas os textos vêm de
 * i18n/<língua>.js, carregado só nesse caso. Os textos criados por JavaScript usam
 * L({ pt, en, fr, es, de, it }) (uma língua que falte fica em inglês).
 */

const SUPPORTED = ['pt', 'en', 'fr', 'es', 'de', 'it'];
const HTML_LANG = { pt: 'pt-PT', en: 'en', fr: 'fr', es: 'es', de: 'de', it: 'it' };

function fromUrl() {
  try {
    const q = new URLSearchParams(location.search).get('lang');
    return SUPPORTED.includes(q) ? q : null;
  } catch {
    return null;
  }
}

function fromBrowser() {
  const list = navigator.languages?.length ? navigator.languages : [navigator.language || 'pt'];
  for (const l of list) {
    const base = String(l).slice(0, 2).toLowerCase();
    if (SUPPORTED.includes(base)) return base;
  }
  return 'en';
}

const chosen = fromUrl();
export const lang = chosen ?? fromBrowser();

/** Texto na língua da página: L({ pt: '...', en: '...', fr: '...', es: '...', de: '...', it: '...' }). */
export const L = (o) => o[lang] ?? o.en ?? o.pt;

/** Formato de números na língua da página. */
export const locale = HTML_LANG[lang];

const loaders = {
  en: () => import('./i18n/en.js'),
  fr: () => import('./i18n/fr.js'),
  es: () => import('./i18n/es.js'),
  de: () => import('./i18n/de.js'),
  it: () => import('./i18n/it.js'),
};

function applyDict(dict) {
  const page = document.documentElement.dataset.page;
  for (const el of document.querySelectorAll('[data-i18n]')) {
    const v = dict[el.dataset.i18n];
    if (v == null) continue;
    if (el instanceof SVGElement) el.textContent = v.replace(/&nbsp;/g, ' ');
    else el.innerHTML = v;
  }
  for (const attr of ['alt', 'aria-label', 'title', 'content', 'placeholder']) {
    for (const el of document.querySelectorAll(`[data-i18n-${attr}]`)) {
      const v = dict[el.getAttribute(`data-i18n-${attr}`)];
      if (v != null) el.setAttribute(attr, v);
    }
  }
  const title = dict[`${page}._title`];
  if (title) document.title = title;
}

/** Os links internos levam a língua escolhida para a página seguinte. */
function keepLangInLinks() {
  if (!chosen) return;
  for (const a of document.querySelectorAll('a[href]')) {
    if (a.closest('.lang')) continue;
    const href = a.getAttribute('href');
    if (!href || /^(https?:|mailto:|#)/.test(href)) continue;
    const url = new URL(href, location.href);
    if (url.origin !== location.origin) continue;
    url.searchParams.set('lang', chosen);
    a.setAttribute('href', url.pathname + url.search + url.hash);
  }
}

/** Seletor PT · EN · FR · ES · DE · IT: marca a língua atual e mantém a página e o #. */
function markSwitcher() {
  for (const a of document.querySelectorAll('.lang a[data-lang]')) {
    const l = a.dataset.lang;
    const url = new URL(location.href);
    url.searchParams.set('lang', l);
    a.setAttribute('href', url.pathname + url.search + url.hash);
    if (l === lang) a.setAttribute('aria-current', 'true');
    else a.removeAttribute('aria-current');
  }
}

/** Traduz a página (chamar uma vez, no arranque de cada página). */
let pending = null;
export function translatePage() {
  pending ??= run();
  return pending;
}

async function run() {
  document.documentElement.lang = HTML_LANG[lang];
  markSwitcher();
  keepLangInLinks();
  if (lang === 'pt') return;
  try {
    const { default: dict } = await loaders[lang]();
    applyDict(dict);
    keepLangInLinks();
  } catch {
    // Sem a tradução, fica o português.
  }
}
