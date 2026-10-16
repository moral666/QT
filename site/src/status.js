/**
 * Marcador de estado do serviço, ao centro da barra de topo.
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
  checking: { short: 'A verificar serviços', long: 'A verificar se os serviços da SEALER estão a funcionar' },
  on: { short: 'Serviços online', long: 'Os serviços da SEALER estão a funcionar' },
  off: { short: 'Serviços offline', long: 'Os serviços da SEALER estão em baixo de momento' },
  unknown: { short: 'Estado desconhecido', long: 'Não foi possível saber o estado dos serviços da SEALER' },
};

export function initStatus(el) {
  if (!el) return;
  const label = el.querySelector('.status-pill__text');
  let timer = 0;

  function show(state) {
    el.dataset.state = state;
    label.textContent = TEXT[state].short;
    const full = `${TEXT[state].long}. ${TESTING}`;
    el.title = full;
    el.setAttribute('aria-label', full);
  }

  async function check() {
    try {
      const res = await fetch('/api/estado', { headers: { Accept: 'application/json' } });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      show(data.estado === 'on' ? 'on' : data.estado === 'off' ? 'off' : 'unknown');
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
