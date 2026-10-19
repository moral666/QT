/**
 * Marcador de estado do serviço, ao lado do nome SEALER na barra de topo.
 *
 * Pergunta ao próprio site (/api/estado, ver worker/index.js) se o relay da SEALER na AWS
 * está a funcionar. Verde: a funcionar. Vermelho: em baixo. Cinzento: ainda a verificar,
 * ou sem resposta do site (nesse caso não sabemos o estado do relay, por isso não se pinta
 * de vermelho).
 *
 * Volta a verificar a cada INTERVAL_MS enquanto a página está visível.
 */

import { L, locale } from './i18n.js';

const INTERVAL_MS = 60_000;

// A app está em testes fechados (versão 0.9.x): o marcador mostra o estado do serviço usado
// pelos testadores, não quer dizer que a app já esteja disponível ao público.
// Número de testadores: alterar aqui e no index.html (status-pill__count).
const TESTING = L({
  pt: 'A SEALER está em testes fechados: por agora só 5 testadores convidados têm acesso ao serviço.',
  en: 'SEALER is in closed testing: for now only 5 invited testers have access to the service.',
  es: 'SEALER está en pruebas cerradas: por ahora solo 5 probadores invitados tienen acceso al servicio.',
});
const TAG = L({
  pt: 'Testes fechados<span class="status-pill__count"> · 5 pessoas</span>',
  en: 'Closed testing<span class="status-pill__count"> · 5 people</span>',
  es: 'Pruebas cerradas<span class="status-pill__count"> · 5 personas</span>',
});

const TEXT = L({
  pt: {
    checking: { short: 'A verificar serviços', mini: 'A verificar', long: 'A verificar se os serviços da SEALER estão a funcionar' },
    on: { short: 'Serviços online', mini: 'Online', long: 'Os serviços da SEALER estão a funcionar' },
    off: { short: 'Serviços offline', mini: 'Offline', long: 'Os serviços da SEALER estão em baixo de momento' },
    unknown: { short: 'Estado desconhecido', mini: 'Sem dados', long: 'Não foi possível saber o estado dos serviços da SEALER' },
  },
  en: {
    checking: { short: 'Checking services', mini: 'Checking', long: 'Checking whether the SEALER services are working' },
    on: { short: 'Services online', mini: 'Online', long: 'The SEALER services are working' },
    off: { short: 'Services offline', mini: 'Offline', long: 'The SEALER services are down at the moment' },
    unknown: { short: 'Status unknown', mini: 'No data', long: 'Could not find out the status of the SEALER services' },
  },
  es: {
    checking: { short: 'Comprobando servicios', mini: 'Comprobando', long: 'Comprobando si los servicios de SEALER funcionan' },
    on: { short: 'Servicios en línea', mini: 'En línea', long: 'Los servicios de SEALER están funcionando' },
    off: { short: 'Servicios caídos', mini: 'Caídos', long: 'Los servicios de SEALER están caídos en este momento' },
    unknown: { short: 'Estado desconocido', mini: 'Sin datos', long: 'No se ha podido saber el estado de los servicios de SEALER' },
  },
});

const W = L({
  pt: {
    how: 'o tempo que uma mensagem típica demora do telemóvel de quem a envia até o telemóvel de quem a recebe confirmar que a tem (envio até ao servidor mais entrega)',
    now: 'agora mesmo', min1: 'há 1 minuto', mins: (n) => `há ${n} minutos`, h1: 'há 1 hora', hs: (n) => `há ${n} horas`, days: (n) => `há ${n} dias`,
    msg1: '1 mensagem', msgs: (n) => `${n} mensagens`,
    hour: (v, n, how) => ` Tempo entre telemóveis na última hora: ${v} ms (${n}), ${how}.`,
    last: (v, when, how) => ` Último tempo entre telemóveis: ${v} ms${when}, ${how}.`,
    measured: (a) => ` (medido ${a})`,
  },
  en: {
    how: 'the time a typical message takes from the sender’s phone until the recipient’s phone confirms it has it (sending to the server plus delivery)',
    now: 'just now', min1: '1 minute ago', mins: (n) => `${n} minutes ago`, h1: '1 hour ago', hs: (n) => `${n} hours ago`, days: (n) => `${n} days ago`,
    msg1: '1 message', msgs: (n) => `${n} messages`,
    hour: (v, n, how) => ` Phone-to-phone time in the last hour: ${v} ms (${n}), ${how}.`,
    last: (v, when, how) => ` Last phone-to-phone time: ${v} ms${when}, ${how}.`,
    measured: (a) => ` (measured ${a})`,
  },
  es: {
    how: 'el tiempo que tarda un mensaje típico desde el móvil de quien lo envía hasta que el móvil de quien lo recibe confirma que lo tiene (envío al servidor más entrega)',
    now: 'ahora mismo', min1: 'hace 1 minuto', mins: (n) => `hace ${n} minutos`, h1: 'hace 1 hora', hs: (n) => `hace ${n} horas`, days: (n) => `hace ${n} días`,
    msg1: '1 mensaje', msgs: (n) => `${n} mensajes`,
    hour: (v, n, how) => ` Tiempo entre móviles en la última hora: ${v} ms (${n}), ${how}.`,
    last: (v, when, how) => ` Último tiempo entre móviles: ${v} ms${when}, ${how}.`,
    measured: (a) => ` (medido ${a})`,
  },
});

export function initStatus(el) {
  if (!el) return;
  const label = el.querySelector('.status-pill__text');
  // Versão curta, só para telemóveis muito estreitos (ver styles.css).
  const mini = el.querySelector('.status-pill__mini');
  const ms = el.querySelector('.status-pill__ms');
  const fmt = new Intl.NumberFormat(locale, { maximumFractionDigits: 0 });
  const tagEl = el.querySelector('.status-pill__tag');
  if (tagEl) tagEl.innerHTML = TAG;
  let timer = 0;

  const HOW = W.how;

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
    if (!data || data.estado !== 'on') return null;
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

  /** "há 5 minutos", "há 3 horas", "há 2 dias". */
  function ago(unixSecs) {
    const min = Math.max(0, Math.round((Date.now() / 1000 - unixSecs) / 60));
    if (min < 1) return W.now;
    if (min < 60) return min === 1 ? W.min1 : W.mins(min);
    const h = Math.round(min / 60);
    if (h < 48) return h === 1 ? W.h1 : W.hs(h);
    return W.days(Math.round(h / 24));
  }

  /** Frase sobre o tempo entre telemóveis, para o title; vazia se não se aplica. */
  function totalText(data) {
    const now = currentValue(data);
    if (now != null) {
      const n = data.mensagens === 1 ? W.msg1 : W.msgs(fmt.format(data.mensagens));
      return W.hour(fmt.format(now), n, HOW);
    }
    const last = lastValue(data);
    if (last != null) {
      const at = Math.max(data.ultimo_envio_medicao || 0, data.ultima_medicao || 0);
      const when = at > 0 ? W.measured(ago(at)) : '';
      return W.last(fmt.format(last), when, HOW);
    }
    return '';
  }

  function show(state, data) {
    el.dataset.state = state;
    label.textContent = TEXT[state].short;
    if (mini) mini.textContent = TEXT[state].mini;
    if (ms) {
      // Fica sempre a última leitura conhecida, mesmo sem mensagens recentes ou com o serviço em baixo.
      const value = totalValue(data);
      if (value != null) {
        ms.hidden = false;
        ms.textContent = `${fmt.format(value)} ms`;
      }
    }
    const full = `${TEXT[state].long}.${totalText(data)} ${TESTING}`;
    el.title = full;
    el.setAttribute('aria-label', full);
  }

  async function check() {
    try {
      const res = await fetch('/api/estado', { headers: { Accept: 'application/json' } });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      show(data.estado === 'on' ? 'on' : data.estado === 'off' ? 'off' : 'unknown', data);
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
