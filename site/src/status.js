/**
 * Marcador de estado do serviço, ao lado do nome SEALER na barra de topo.
 *
 * Pergunta ao próprio site (/api/estado, ver worker/index.js) se o relay da SEALER na AWS
 * está a funcionar, e cada relay (Suécia e Espanha) tem o seu ponto. Verde: a funcionar.
 * Amarelo: só parte dos relays a funcionar. Vermelho: em baixo. Cinzento: ainda a verificar,
 * ou sem resposta do site (nesse caso não sabemos o estado do relay, por isso não se pinta
 * de vermelho).
 *
 * Volta a verificar a cada INTERVAL_MS enquanto a página está visível.
 */

import { L, locale } from './i18n.js';

const INTERVAL_MS = 60_000;

// A app está em acesso antecipado, por convite: o marcador mostra o estado do serviço usado
// por quem já foi convidado, não quer dizer que a app já esteja nas lojas.
// Fase do serviço: alterar aqui e no index.html (status-pill__tag).
const TESTING = L({
  pt: 'A SEALER está em acesso antecipado: por agora entra-se por convite.',
  en: 'SEALER is in early access: for now, entry is by invitation.',
  fr: 'SEALER est en accès anticipé : pour l’instant, on y entre sur invitation.',
  es: 'SEALER está en acceso anticipado: por ahora se entra por invitación.',
  de: 'SEALER ist im Early Access: Der Zugang erfolgt vorerst auf Einladung.',
  it: 'SEALER è in accesso anticipato: per ora si entra su invito.',
  ja: 'SEALERは先行アクセス中です。現在は招待制で利用できます。',
  zh: 'SEALER正处于抢先体验阶段：目前需凭邀请加入。',
  hi: 'SEALER अर्ली एक्सेस में है: अभी इसमें सिर्फ़ आमंत्रण से शामिल हुआ जा सकता है।',
  ar: 'تطبيق SEALER في مرحلة الوصول المبكر: الانضمام إليه حاليًا بدعوة فقط.',
});
const TAG = L({
  pt: 'Acesso por convite',
  en: 'Invitation only',
  fr: 'Sur invitation',
  es: 'Acceso por invitación',
  de: 'Zugang per Einladung',
  it: 'Accesso su invito',
  ja: '招待制',
  zh: '凭邀请加入',
  hi: 'आमंत्रण से प्रवेश',
  ar: 'الوصول بدعوة',
});

const TEXT = L({
  pt: {
    checking: { short: 'A verificar serviços', mini: 'A verificar', long: 'A verificar se os serviços da SEALER estão a funcionar' },
    on: { short: 'Serviços online', mini: 'Online', long: 'Os serviços da SEALER estão a funcionar' },
    off: { short: 'Serviços offline', mini: 'Offline', long: 'Os serviços da SEALER estão em baixo de momento' },
    partial: { short: 'Serviços parciais', mini: 'Parcial', long: 'Parte dos serviços da SEALER está em baixo de momento' },
    unknown: { short: 'Estado desconhecido', mini: 'Sem dados', long: 'Não foi possível saber o estado dos serviços da SEALER' },
  },
  en: {
    checking: { short: 'Checking services', mini: 'Checking', long: 'Checking whether the SEALER services are working' },
    on: { short: 'Services online', mini: 'Online', long: 'The SEALER services are working' },
    off: { short: 'Services offline', mini: 'Offline', long: 'The SEALER services are down at the moment' },
    partial: { short: 'Partial outage', mini: 'Partial', long: 'Part of the SEALER services is down at the moment' },
    unknown: { short: 'Status unknown', mini: 'No data', long: 'Could not find out the status of the SEALER services' },
  },
  fr: {
    checking: { short: 'Vérification des services', mini: 'Vérification', long: 'Vérification du bon fonctionnement des services de SEALER' },
    on: { short: 'Services en ligne', mini: 'En ligne', long: 'Les services de SEALER fonctionnent' },
    off: { short: 'Services hors ligne', mini: 'Hors ligne', long: 'Les services de SEALER sont indisponibles pour le moment' },
    partial: { short: 'Panne partielle', mini: 'Partiel', long: 'Une partie des services de SEALER est indisponible pour le moment' },
    unknown: { short: 'État inconnu', mini: 'Pas de données', long: 'Impossible de connaître l’état des services de SEALER' },
  },
  de: {
    checking: { short: 'Dienste werden geprüft', mini: 'Prüfung', long: 'Es wird geprüft, ob die Dienste von SEALER funktionieren' },
    on: { short: 'Dienste online', mini: 'Online', long: 'Die Dienste von SEALER funktionieren' },
    off: { short: 'Dienste offline', mini: 'Offline', long: 'Die Dienste von SEALER sind derzeit nicht erreichbar' },
    partial: { short: 'Teilweise gestört', mini: 'Teilweise', long: 'Ein Teil der Dienste von SEALER ist derzeit nicht erreichbar' },
    unknown: { short: 'Status unbekannt', mini: 'Keine Daten', long: 'Der Status der Dienste von SEALER konnte nicht ermittelt werden' },
  },
  it: {
    checking: { short: 'Verifica dei servizi', mini: 'Verifica', long: 'Verifica del funzionamento dei servizi di SEALER' },
    on: { short: 'Servizi online', mini: 'Online', long: 'I servizi di SEALER funzionano' },
    off: { short: 'Servizi offline', mini: 'Offline', long: 'I servizi di SEALER sono al momento non disponibili' },
    partial: { short: 'Disservizio parziale', mini: 'Parziale', long: 'Una parte dei servizi di SEALER è al momento non disponibile' },
    unknown: { short: 'Stato sconosciuto', mini: 'Nessun dato', long: 'Impossibile conoscere lo stato dei servizi di SEALER' },
  },
  es: {
    checking: { short: 'Comprobando servicios', mini: 'Comprobando', long: 'Comprobando si los servicios de SEALER funcionan' },
    on: { short: 'Servicios en línea', mini: 'En línea', long: 'Los servicios de SEALER están funcionando' },
    off: { short: 'Servicios caídos', mini: 'Caídos', long: 'Los servicios de SEALER están caídos en este momento' },
    partial: { short: 'Caída parcial', mini: 'Parcial', long: 'Parte de los servicios de SEALER está caída en este momento' },
    unknown: { short: 'Estado desconocido', mini: 'Sin datos', long: 'No se ha podido saber el estado de los servicios de SEALER' },
  },
  ja: {
    checking: { short: 'サービスを確認中', mini: '確認中', long: 'SEALERのサービスが動作しているか確認しています' },
    on: { short: 'サービス稼働中', mini: '稼働中', long: 'SEALERのサービスは正常に動作しています' },
    off: { short: 'サービス停止中', mini: '停止中', long: 'SEALERのサービスは現在停止しています' },
    partial: { short: 'サービス一部停止', mini: '一部停止', long: 'SEALERのサービスの一部が現在停止しています' },
    unknown: { short: '状態不明', mini: 'データなし', long: 'SEALERのサービスの状態を確認できませんでした' },
  },
  zh: {
    checking: { short: '正在检查服务', mini: '检查中', long: '正在检查SEALER的服务是否正常运行' },
    on: { short: '服务在线', mini: '在线', long: 'SEALER的服务运行正常' },
    off: { short: '服务离线', mini: '离线', long: 'SEALER的服务目前已中断' },
    partial: { short: '部分服务中断', mini: '部分中断', long: 'SEALER的部分服务目前已中断' },
    unknown: { short: '状态未知', mini: '无数据', long: '无法获取SEALER服务的状态' },
  },
  hi: {
    checking: { short: 'सेवाएं जांची जा रही हैं', mini: 'जांच जारी', long: 'जांच की जा रही है कि SEALER की सेवाएं काम कर रही हैं या नहीं' },
    on: { short: 'सेवाएं ऑनलाइन', mini: 'ऑनलाइन', long: 'SEALER की सेवाएं काम कर रही हैं' },
    off: { short: 'सेवाएं ऑफ़लाइन', mini: 'ऑफ़लाइन', long: 'SEALER की सेवाएं अभी बंद हैं' },
    partial: { short: 'सेवाएं आंशिक रूप से बंद', mini: 'आंशिक', long: 'SEALER की कुछ सेवाएं अभी बंद हैं' },
    unknown: { short: 'स्थिति अज्ञात', mini: 'डेटा नहीं', long: 'SEALER की सेवाओं की स्थिति पता नहीं चल सकी' },
  },
  ar: {
    checking: { short: 'جارٍ فحص الخدمات', mini: 'جارٍ الفحص', long: 'جارٍ التحقق من عمل خدمات SEALER' },
    on: { short: 'الخدمات تعمل', mini: 'تعمل', long: 'خدمات SEALER تعمل' },
    off: { short: 'الخدمات متوقفة', mini: 'متوقفة', long: 'خدمات SEALER متوقفة حاليًا' },
    partial: { short: 'توقف جزئي', mini: 'جزئي', long: 'جزء من خدمات SEALER متوقف حاليًا' },
    unknown: { short: 'الحالة غير معروفة', mini: 'لا بيانات', long: 'تعذّرت معرفة حالة خدمات SEALER' },
  },
});

/**
 * Nome do país de cada relay (o "nome" que vem em /api/estado, igual ao das apps).
 * Relay novo: acrescentar aqui o país nas dez línguas e o relay em RELAYS (wrangler.jsonc).
 */
const RELAY_NAMES = L({
  pt: { estocolmo: 'Suécia', espanha: 'Espanha' },
  en: { estocolmo: 'Sweden', espanha: 'Spain' },
  fr: { estocolmo: 'Suède', espanha: 'Espagne' },
  es: { estocolmo: 'Suecia', espanha: 'España' },
  de: { estocolmo: 'Schweden', espanha: 'Spanien' },
  it: { estocolmo: 'Svezia', espanha: 'Spagna' },
  ja: { estocolmo: 'スウェーデン', espanha: 'スペイン' },
  zh: { estocolmo: '瑞典', espanha: '西班牙' },
  hi: { estocolmo: 'स्वीडन', espanha: 'स्पेन' },
  ar: { estocolmo: 'السويد', espanha: 'إسبانيا' },
});

/** Nome a mostrar de um relay; um relay sem nome traduzido fica com o próprio nome. */
function relayName(nome) {
  return RELAY_NAMES[nome] ?? nome.charAt(0).toUpperCase() + nome.slice(1);
}

/**
 * O que são os milissegundos ao lado de cada relay (na frase completa). code: o centro de
 * dados de onde foram medidos (ex.: LIS), ou vazio.
 */
const PING_TEXT = L({
  pt: (c) => `Os milissegundos são o tempo de ida e volta até cada servidor, medido a partir do centro de dados do site mais perto de ti${c ? ` (${c})` : ''}.`,
  en: (c) => `The milliseconds are the round-trip time to each server, measured from the site’s data centre closest to you${c ? ` (${c})` : ''}.`,
  fr: (c) => `Les millisecondes sont le temps aller-retour jusqu’à chaque serveur, mesuré depuis le centre de données du site le plus proche de toi${c ? ` (${c})` : ''}.`,
  es: (c) => `Los milisegundos son el tiempo de ida y vuelta hasta cada servidor, medido desde el centro de datos del sitio más cercano a ti${c ? ` (${c})` : ''}.`,
  de: (c) => `Die Millisekunden sind die Hin- und Rücklaufzeit zu jedem Server, gemessen vom Rechenzentrum der Website, das dir am nächsten ist${c ? ` (${c})` : ''}.`,
  it: (c) => `I millisecondi sono il tempo di andata e ritorno verso ciascun server, misurato dal data center del sito più vicino a te${c ? ` (${c})` : ''}.`,
  ja: (c) => `ミリ秒は各サーバーまでの往復時間で、あなたに最も近いサイトのデータセンター${c ? `（${c}）` : ''}から測定しています。`,
  zh: (c) => `毫秒数是到每台服务器的往返时间，从离你最近的网站数据中心${c ? `（${c}）` : ''}测得。`,
  hi: (c) => `मिलीसेकंड हर सर्वर तक आने-जाने का समय हैं, जिसे आपके सबसे पास के साइट के डेटा सेंटर${c ? ` (${c})` : ''} से मापा गया है।`,
  ar: (c) => `أجزاء الثانية هي زمن الذهاب والإياب إلى كل خادم، مقيسًا من أقرب مركز بيانات للموقع إليك${c ? ` (\u2066${c}\u2069)` : ''}.`,
});

/** Pontuação entre frases: japonês e chinês com 。 e sem espaços; hindi com ।. */
const W = L({
  pt: {}, en: {}, fr: {}, es: {}, de: {}, it: {}, ar: {},
  ja: { stop: '。', sep: '' },
  zh: { stop: '。', sep: '' },
  hi: { stop: '।' },
});

export function initStatus(el) {
  if (!el) return;
  const label = el.querySelector('.status-pill__text');
  // Versão curta, só para telemóveis muito estreitos (ver styles.css).
  const mini = el.querySelector('.status-pill__mini');
  const ms = el.querySelector('.status-pill__ms');
  // Um ponto por relay (Suécia, Espanha), na segunda linha.
  const relaysEl = el.querySelector('.status-pill__relays');
  const fmt = new Intl.NumberFormat(locale, { maximumFractionDigits: 0 });
  const tagEl = el.querySelector('.status-pill__tag');
  if (tagEl) tagEl.innerHTML = TAG;
  let timer = 0;

  /** Envio (telemóvel -> servidor) + entrega (servidor -> destinatário confirmar), em ms. */
  function sum(a, b) {
    return Number.isFinite(a) && Number.isFinite(b) ? a + b : null;
  }

  /** A mediana (a mensagem típica) quando existe; senão a média (relay antigo). */
  function pick(median, avg) {
    return Number.isFinite(median) ? median : avg;
  }

  /** Mensagem típica da última hora (só com o serviço a funcionar e as duas partes medidas). */
  function currentValue(data) {
    if (!data || (data.estado !== 'on' && data.estado !== 'partial')) return null;
    return sum(pick(data.envio_mediana_ms, data.envio_media_ms), pick(data.entrega_mediana_ms, data.entrega_media_ms));
  }

  /** Última medição conhecida, mesmo antiga. */
  function lastValue(data) {
    if (!data) return null;
    return sum(pick(data.ultimo_envio_mediana_ms, data.ultimo_envio_ms), pick(data.ultima_mediana_ms, data.ultima_media_ms));
  }

  /** Valor a mostrar: a mensagem típica da última hora ou, sem mensagens recentes, a última medição. */
  function totalValue(data) {
    return currentValue(data) ?? lastValue(data);
  }

  /** "77 ms" isolado (numa página em árabe fica "77 ms" e não "ms 77"); vazio sem medição. */
  function pingLabel(r) {
    return Number.isFinite(r?.ping_ms) ? `\u2066${fmt.format(r.ping_ms)} ms\u2069` : '';
  }

  function show(state, data) {
    el.dataset.state = state;
    label.textContent = TEXT[state].short;
    if (mini) mini.textContent = TEXT[state].mini;
    // O quadro "Entre telemóveis" (Testes e números) continua com o tempo entre telemóveis, no
    // original e nas cópias que a grelha faz em ecrãs grandes (sem ids, por isso data-live).
    // Fica sempre a última leitura conhecida, mesmo sem mensagens recentes.
    const value = totalValue(data);
    if (value != null) {
      for (const tile of document.querySelectorAll('[data-live="tempo"]')) {
        tile.textContent = `\u2066${fmt.format(value)} ms\u2069`;
      }
    }
    const relays = Array.isArray(data?.relays) ? data.relays.filter((r) => r && typeof r.nome === 'string') : [];
    // No marcador: o ping até cada relay, ao lado do seu ponto (segunda linha). Nos telemóveis
    // estreitos essa linha não aparece: ao lado do estado fica só o do relay mais perto.
    if (ms) {
      const pings = relays.map((r) => r.ping_ms).filter((v) => Number.isFinite(v));
      if (pings.length > 0) {
        ms.hidden = false;
        ms.textContent = `\u2066${fmt.format(Math.min(...pings))} ms\u2069`;
      }
    }
    if (relaysEl) {
      relaysEl.replaceChildren(
        ...relays.map((r) => {
          const item = document.createElement('span');
          item.className = 'status-relay';
          item.dataset.state = r.estado === 'on' ? 'on' : 'off';
          const dot = document.createElement('span');
          dot.className = 'status-relay__dot';
          dot.setAttribute('aria-hidden', 'true');
          item.append(dot, relayName(r.nome));
          const ping = r.estado === 'on' ? pingLabel(r) : '';
          if (ping) {
            const p = document.createElement('span');
            p.className = 'status-relay__ms';
            p.textContent = ping;
            item.append(p);
          }
          return item;
        }),
      );
      relaysEl.hidden = relays.length === 0;
    }
    // "Suécia: Online. Espanha: Online." (na frase completa, para leitores de ecrã e title).
    // "Suécia: Online (77 ms)."
    const relaysText = relays
      .map((r) => {
        const ping = r.estado === 'on' ? pingLabel(r) : '';
        return `${relayName(r.nome)}: ${TEXT[r.estado === 'on' ? 'on' : 'off'].mini}${ping ? ` (${ping})` : ''}${W.stop ?? '.'}`;
      })
      .join(W.sep ?? ' ');
    const hasPing = relays.some((r) => r.estado === 'on' && Number.isFinite(r.ping_ms));
    const code = typeof data?.medido_em === 'string' ? data.medido_em.replace(/[^A-Z0-9]/gi, '').slice(0, 6) : '';
    const pingText = hasPing ? (W.sep ?? ' ') + PING_TEXT(code) : '';
    // Japones e chines: ponto final 。 e sem espacos entre frases; hindi: ponto final ।.
    const full = `${TEXT[state].long}${W.stop ?? '.'}${relaysText ? (W.sep ?? ' ') + relaysText : ''}${pingText}${W.sep ?? ' '}${TESTING}`;
    el.title = full;
    el.setAttribute('aria-label', full);
  }

  async function check() {
    try {
      const res = await fetch('/api/estado', { headers: { Accept: 'application/json' } });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      const st = data.estado;
      show(st === 'on' || st === 'partial' || st === 'off' ? st : 'unknown', data);
    } catch {
      show('unknown');
    }
  }

  function schedule() {
    clearInterval(timer);
    if (document.visibilityState === 'visible') timer = setInterval(check, INTERVAL_MS);
  }

  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') check();
    schedule();
  });

  show('checking');
  el.hidden = false;
  check();
  schedule();
}
