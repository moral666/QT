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
 *   Resposta: { "estado": "on" | "off", "verificado": "<data ISO>" }
 */

const CACHE_SECONDS = 30;
const TIMEOUT_MS = 5000;

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

  const on = await relayOnline(env.RELAY_URL);
  const response = json({ estado: on ? 'on' : 'off', verificado: new Date().toISOString() }, 200, `public, max-age=${CACHE_SECONDS}`);
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
