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

// A app está em testes fechados (alpha): o marcador mostra o estado do serviço usado
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

  /** Frase sobre o tempo de entrega, para o title; vazia se não se aplica. */
  function deliveryText(data) {
    if (!data || data.estado !== 'on') return '';
    const when = data.janela_minutos >= 60 || !data.janela_minutos ? 'na última hora' : `nos últimos ${data.janela_minutos} minutos`;
    if (data.entrega_media_ms == null) return ` Sem mensagens entregues ${when}.`;
    const n = data.mensagens === 1 ? '1 mensagem' : `${fmt.format(data.mensagens)} mensagens`;
    return ` Tempo médio de entrega ${when}: ${fmt.format(data.entrega_media_ms)} ms (${n}), desde que o servidor recebe a mensagem até o telemóvel de quem a recebe confirmar que a tem.`;
  }

  function show(state, data) {
    el.dataset.state = state;
    label.textContent = TEXT[state].short;
    if (mini) mini.textContent = TEXT[state].mini;
    if (ms) {
      const value = state === 'on' && data && Number.isFinite(data.entrega_media_ms) ? data.entrega_media_ms : null;
      ms.hidden = value == null;
      ms.textContent = value == null ? '' : `${fmt.format(value)} ms`;
    }
    const full = `${TEXT[state].long}.${deliveryText(data)} ${TESTING}`;
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
