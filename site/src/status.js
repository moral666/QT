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

const INTERVAL_MS = 60_000;

// A app está em testes fechados (versão 0.7): o marcador mostra o estado do serviço usado
// pelos testadores, não quer dizer que a app já esteja disponível ao público.
// Número de testadores: alterar aqui e no index.html (status-pill__count).
const TESTING = 'A SEALER está em testes fechados: por agora só 5 testadores convidados têm acesso ao serviço.';

const TEXT = {
  checking: { short: 'A verificar serviços', mini: 'A verificar', long: 'A verificar se os serviços da SEALER estão a funcionar' },
  on: { short: 'Serviços online', mini: 'Online', long: 'Os serviços da SEALER estão a funcionar' },
  off: { short: 'Serviços offline', mini: 'Offline', long: 'Os serviços da SEALER estão em baixo de momento' },
  unknown: { short: 'Estado desconhecido', mini: 'Sem dados', long: 'Não foi possível saber o estado dos serviços da SEALER' },
};

export function initStatus(el) {
  if (!el) return;
  const label = el.querySelector('.status-pill__text');
  // Versão curta, só para telemóveis muito estreitos (ver styles.css).
  const mini = el.querySelector('.status-pill__mini');
  const ms = el.querySelector('.status-pill__ms');
  const fmt = new Intl.NumberFormat('pt-PT', { maximumFractionDigits: 0 });
  let timer = 0;

  const HOW = 'o tempo que uma mensagem demora a ir do telemóvel de quem a envia até ao servidor da SEALER';

  /** Valor a mostrar: a média da última hora ou, sem mensagens recentes, a última medição. */
  function sendValue(data) {
    if (!data) return null;
    if (data.estado === 'on' && Number.isFinite(data.envio_media_ms)) return data.envio_media_ms;
    return Number.isFinite(data.ultimo_envio_ms) ? data.ultimo_envio_ms : null;
  }

  /** "há 5 minutos", "há 3 horas", "há 2 dias". */
  function ago(unixSecs) {
    const min = Math.max(0, Math.round((Date.now() / 1000 - unixSecs) / 60));
    if (min < 1) return 'agora mesmo';
    if (min < 60) return min === 1 ? 'há 1 minuto' : `há ${min} minutos`;
    const h = Math.round(min / 60);
    if (h < 48) return h === 1 ? 'há 1 hora' : `há ${h} horas`;
    return `há ${Math.round(h / 24)} dias`;
  }

  /** Frase sobre o tempo de envio, para o title; vazia se não se aplica. */
  function sendText(data) {
    if (!data) return '';
    if (data.estado === 'on' && Number.isFinite(data.envio_media_ms)) {
      const n = data.envios === 1 ? '1 mensagem' : `${fmt.format(data.envios)} mensagens`;
      return ` Tempo médio de envio na última hora: ${fmt.format(data.envio_media_ms)} ms (${n}), ${HOW}.`;
    }
    if (Number.isFinite(data.ultimo_envio_ms)) {
      const when = Number.isFinite(data.ultimo_envio_medicao) ? ` (medido ${ago(data.ultimo_envio_medicao)})` : '';
      return ` Último tempo médio de envio: ${fmt.format(data.ultimo_envio_ms)} ms${when}, ${HOW}.`;
    }
    return '';
  }

  function show(state, data) {
    el.dataset.state = state;
    label.textContent = TEXT[state].short;
    if (mini) mini.textContent = TEXT[state].mini;
    if (ms) {
      // Fica sempre a última leitura conhecida, mesmo sem mensagens recentes ou com o serviço em baixo.
      const value = sendValue(data);
      if (value != null) {
        ms.hidden = false;
        ms.textContent = `${fmt.format(value)} ms`;
      }
    }
    const full = `${TEXT[state].long}.${sendText(data)} ${TESTING}`;
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
