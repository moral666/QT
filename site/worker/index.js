/**
 * Worker "sealer" da Cloudflare.
 *
 * Quase tudo são ficheiros estáticos (pasta dist/), servidos pela Cloudflare sem passar
 * por aqui. Só os pedidos a /api/* correm este código (ver run_worker_first em wrangler.jsonc).
 *
 * GET /api/estado
 *   Diz se o serviço da SEALER na AWS (o relay) está a funcionar. O worker tenta abrir uma
 *   ligação WebSocket ao relay, como faria a app, e fecha-a logo a seguir. O resultado fica
 *   guardado durante CACHE_SECONDS, por isso o relay recebe no máximo uma verificação por
 *   esse intervalo e por centro de dados da Cloudflare, por mais visitantes que haja.
 *   O navegador do visitante nunca fala com a AWS: só com o próprio site.
 *
 *   Também lê https://<relay>/estado.json (escrito pelo relay a cada minuto, ver
 *   server/src/latency.rs): o tempo médio de entrega das mensagens na última hora.
 *
 *   Resposta: {
 *     "estado": "on" | "off", "verificado": "<data ISO>",
 *     "entrega_media_ms": número | null,   (null: sem mensagens, ou relay sem dados recentes)
 *     "mensagens": número, "janela_minutos": número,
 *     "ultima_media_ms": número | null,    (última média conhecida, mesmo antiga: o site mostra-a sempre)
 *     "ultima_medicao": número | null      (quando foi medida, segundos desde 1970)
 *   }
 */

const CACHE_SECONDS = 30;
const TIMEOUT_MS = 5000;
/** O estado.json é reescrito a cada minuto; mais velho do que isto, já não vale. */
const STATUS_MAX_AGE_S = 180;

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    if (url.pathname === '/api/estado') return estado(request, env, ctx);
    if (url.pathname.startsWith('/api/')) return json({ erro: 'não encontrado' }, 404, 'no-store');
    return env.ASSETS.fetch(request);
  },
};

async function estado(request, env, ctx) {
  if (request.method !== 'GET' && request.method !== 'HEAD') {
    return json({ erro: 'método não permitido' }, 405, 'no-store');
  }

  // A cache da Cloudflare não funciona em *.workers.dev; lá, cada pedido verifica o relay.
  const cache = caches.default;
  const key = new Request(new URL('/api/estado', request.url).toString(), { method: 'GET' });
  const cached = await cache.match(key);
  if (cached) return cached;

  const [on, delivery] = await Promise.all([relayOnline(env.RELAY_URL), relayDelivery(env.RELAY_URL)]);
  const body = {
    estado: on ? 'on' : 'off',
    verificado: new Date().toISOString(),
    entrega_media_ms: on && delivery ? delivery.avg : null,
    mensagens: on && delivery ? delivery.count : 0,
    janela_minutos: on && delivery ? delivery.minutes : 0,
    ultima_media_ms: delivery ? delivery.lastAvg : null,
    ultima_medicao: delivery ? delivery.lastAt : null,
  };
  const response = json(body, 200, `public, max-age=${CACHE_SECONDS}`);
  ctx.waitUntil(cache.put(key, response.clone()));
  return response;
}

/** Tenta abrir uma ligação WebSocket ao relay (pelo Caddy, com TLS) e fecha-a logo. */
async function relayOnline(relayUrl) {
  if (!relayUrl) return false;
  try {
    const res = await fetch(relayUrl, {
      headers: { Upgrade: 'websocket' },
      signal: AbortSignal.timeout(TIMEOUT_MS),
    });
    const socket = res.webSocket;
    if (res.status !== 101 || !socket) return false;
    socket.accept();
    socket.close(1000, 'verificacao de estado');
    return true;
  } catch {
    return false;
  }
}

/** Lê o tempo médio de entrega publicado pelo relay (atual e última medição); null se não conseguir. */
async function relayDelivery(relayUrl) {
  if (!relayUrl) return null;
  try {
    const res = await fetch(new URL('/estado.json', relayUrl), {
      headers: { Accept: 'application/json' },
      signal: AbortSignal.timeout(TIMEOUT_MS),
    });
    if (!res.ok) return null;
    const d = await res.json();
    const num = (v) => (Number.isFinite(v) && v >= 0 ? Math.round(v) : null);
    // A última medição vale sempre (é a última conhecida); a média atual só se o ficheiro for recente.
    const lastAvg = num(d.ultima_media_ms);
    const lastAt = lastAvg == null ? null : num(d.ultima_medicao);
    const fresh = typeof d.atualizado === 'number' && Date.now() / 1000 - d.atualizado < STATUS_MAX_AGE_S;
    return {
      avg: fresh ? num(d.entrega_media_ms) : null,
      count: fresh ? num(d.mensagens) ?? 0 : 0,
      minutes: fresh ? num(d.janela_minutos) ?? 0 : 0,
      lastAvg,
      lastAt,
    };
  } catch {
    return null;
  }
}

function json(body, status, cacheControl) {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      'Content-Type': 'application/json; charset=utf-8',
      'Cache-Control': cacheControl,
      'X-Content-Type-Options': 'nosniff',
    },
  });
}
