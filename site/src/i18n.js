/**
 * Línguas do site: as mesmas dez da app. Português (o texto das páginas), inglês,
 * francês, espanhol, alemão, italiano, japonês, chinês simplificado, hindi e árabe.
 * O árabe escreve-se da direita para a esquerda: <html dir="rtl"> e, no CSS, propriedades
 * lógicas (margin-inline-start…) em vez de esquerda/direita.
 *
 * Escolha: `?lang=pt|en|fr|es|de|it|ja|zh|hi|ar` no endereço (o menu de línguas põe-no); sem
 * isso, a língua do navegador (uma destas; outra qualquer: inglês; chinês tradicional, de
 * Taiwan, Hong Kong ou Macau, também inglês, porque o site só tem o simplificado). Nada é guardado no
 * navegador (a política de cookies diz que o site não usa armazenamento): a escolha
 * passa de página em página pelos links, com o `?lang=`.
 *
 * As páginas marcam os textos com data-i18n="chave" (e data-i18n-alt, -aria-label,
 * -title, -content para atributos). Nas outras línguas os textos vêm de
 * i18n/<língua>.js, carregado só nesse caso. Os textos criados por JavaScript usam
 * L({ pt, en, fr, es, de, it, ja, zh, hi, ar }) (uma língua que falte fica em inglês).
 *
 * Nas línguas traduzidas, o rodapé ganha uma linha para avisar de erros de tradução
 * (email para privacidade@sealerapp.com, com a língua e a página no assunto).
 */

const SUPPORTED = ['pt', 'en', 'fr', 'es', 'de', 'it', 'ja', 'zh', 'hi', 'ar'];
const HTML_LANG = { pt: 'pt-PT', en: 'en', fr: 'fr', es: 'es', de: 'de', it: 'it', ja: 'ja', zh: 'zh-Hans', hi: 'hi', ar: 'ar' };
/** Línguas escritas da direita para a esquerda. */
const RTL = new Set(['ar']);
/** Nome de cada língua na própria língua (menu de línguas). */
const NAMES = { pt: 'Português', en: 'English', fr: 'Français', es: 'Español', de: 'Deutsch', it: 'Italiano', ja: '日本語', zh: '简体中文', hi: 'हिन्दी', ar: 'العربية' };

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
    const tag = String(l).toLowerCase();
    // Chinês tradicional: o site só tem o simplificado, por isso passa à língua seguinte.
    if (/^zh-(hant|tw|hk|mo)\b/.test(tag)) continue;
    const base = tag.slice(0, 2);
    if (SUPPORTED.includes(base)) return base;
  }
  return 'en';
}

const chosen = fromUrl();
export const lang = chosen ?? fromBrowser();

/** Texto na língua da página: L({ pt: '...', en: '...', ..., ja: '...', zh: '...', hi: '...' }). */
export const L = (o) => o[lang] ?? o.en ?? o.pt;

/** Formato de números na língua da página (em árabe com algarismos 0-9, decisão do Nuno). */
export const locale = lang === 'ar' ? 'ar-u-nu-latn' : HTML_LANG[lang];

const loaders = {
  en: () => import('./i18n/en.js'),
  fr: () => import('./i18n/fr.js'),
  es: () => import('./i18n/es.js'),
  de: () => import('./i18n/de.js'),
  it: () => import('./i18n/it.js'),
  ja: () => import('./i18n/ja.js'),
  zh: () => import('./i18n/zh.js'),
  hi: () => import('./i18n/hi.js'),
  ar: () => import('./i18n/ar.js'),
};

/**
 * Linha no fim de cada página traduzida para avisar de erros de tradução por email
 * (o português é o original, por isso não a tem). Vai só a língua e a página.
 */
const REPORT = {
  en: 'Found a translation error? <a href="%s">Let us know</a>.',
  fr: 'Une erreur de traduction ? <a href="%s">Dis-le-nous</a>.',
  es: '¿Has visto un error de traducción? <a href="%s">Avísanos</a>.',
  de: 'Einen Übersetzungsfehler gefunden? <a href="%s">Sag uns Bescheid</a>.',
  it: 'Hai trovato un errore di traduzione? <a href="%s">Faccelo sapere</a>.',
  ja: '翻訳の誤りを見つけた場合は、<a href="%s">お知らせください</a>。',
  zh: '发现翻译错误？<a href="%s">请告诉我们</a>。',
  hi: 'अनुवाद में कोई गलती दिखी? <a href="%s">हमें बताएं</a>।',
  ar: 'هل وجدت خطأ في الترجمة؟ <a href="%s">يُرجى إبلاغنا</a>.',
};

function addReportLine() {
  const text = REPORT[lang];
  const footer = document.querySelector('.legal-footer, .footer');
  if (!text || !footer || footer.querySelector('.i18n-report')) return;
  const page = document.documentElement.dataset.page || 'index';
  const href = 'mailto:privacidade@sealerapp.com?subject=' + encodeURIComponent(`SEALER: erro de tradução (site, ${lang}, ${page})`);
  const p = document.createElement('p');
  p.className = 'i18n-report';
  p.innerHTML = text.replace('%s', href);
  footer.append(p);
}

/** O menu de línguas fecha ao tocar fora dele ou com Esc. */
function closeLangMenuOutside() {
  const box = document.querySelector('details.lang');
  if (!box) return;
  document.addEventListener('click', (e) => {
    if (box.open && !box.contains(e.target)) box.open = false;
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && box.open) {
      box.open = false;
      box.querySelector('summary')?.focus();
    }
  });
}

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

/** Menu de línguas: marca a língua atual e mantém a página e o #. */
function markSwitcher() {
  for (const cur of document.querySelectorAll('.lang .lang__current')) {
    cur.textContent = NAMES[lang];
    cur.setAttribute('lang', HTML_LANG[lang]);
  }
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
  document.documentElement.dir = RTL.has(lang) ? 'rtl' : 'ltr';
  markSwitcher();
  closeLangMenuOutside();
  keepLangInLinks();
  if (lang === 'pt') return;
  addReportLine();
  try {
    const { default: dict } = await loaders[lang]();
    applyDict(dict);
    keepLangInLinks();
  } catch {
    // Sem a tradução, fica o português.
  }
}
