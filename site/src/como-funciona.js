/**
 * Página "Como funciona": animação em 5 passos de como viaja uma mensagem.
 *
 * O desenho é um só <svg> (como-funciona.html). Em ecrãs largos, Ana, servidor e Rui
 * ficam lado a lado; em telemóveis, em coluna (LAYOUTS). Cada passo põe data-scene no
 * <svg> (o CSS trata do foco e do que aparece) e corre a sua parte animada aqui.
 *
 * Os textos dos passos estão em SCENES. Têm de continuar verdadeiros em relação à app:
 *   passo 3: as apps enviam com Sealed Sender (o envelope no relay não diz quem enviou);
 *   passo 5: as apps são compiladas com PQXDH (feature "pq" do qt_ffi).
 *
 * Com "reduzir movimento" ligado no sistema, não avança sozinho e não há animações.
 */
import { L, translatePage } from './i18n.js';

const SCENES = [
  {
    title: L({ pt: 'A Ana escreve ao Rui', en: 'Ana writes to Rui', fr: 'Ana écrit à Rui', es: 'Ana le escribe a Rui', de: 'Ana schreibt Rui', it: 'Ana scrive a Rui' }),
    text: L({
      pt: 'Escreve «Olá, Rui!» e carrega em enviar.',
      en: 'She types “Hi, Rui!” and taps send.',
      fr: 'Elle écrit « Salut, Rui ! » et appuie sur envoyer.',
      es: 'Escribe «¡Hola, Rui!» y pulsa enviar.',
      de: 'Sie schreibt „Hallo, Rui!“ und tippt auf Senden.',
      it: 'Scrive «Ciao, Rui!» e tocca invia.',
    }),
    ms: 4500,
  },
  {
    title: L({ pt: 'A mensagem é trancada', en: 'The message is locked', fr: 'Le message est verrouillé', es: 'El mensaje se cierra con llave', de: 'Die Nachricht wird verschlossen', it: 'Il messaggio viene chiuso' }),
    text: L({
      pt: 'Ainda no telemóvel da Ana, a mensagem fica ilegível e leva um cadeado. Só a chave do Rui o abre.',
      en: 'Still on Ana’s phone, the message becomes unreadable and gets a padlock. Only Rui’s key opens it.',
      fr: 'Encore sur le téléphone d’Ana, le message devient illisible et reçoit un cadenas. Seule la clé de Rui l’ouvre.',
      es: 'Todavía en el móvil de Ana, el mensaje se vuelve ilegible y lleva un candado. Solo la llave de Rui lo abre.',
      de: 'Noch auf Anas Handy wird die Nachricht unlesbar und bekommt ein Schloss. Nur Ruis Schlüssel öffnet es.',
      it: 'Ancora sul telefono di Ana, il messaggio diventa illeggibile e riceve un lucchetto. Solo la chiave di Rui lo apre.',
    }),
    ms: 4500,
  },
  {
    title: L({ pt: 'O servidor só a transporta', en: 'The server only carries it', fr: 'Le serveur ne fait que le transporter', es: 'El servidor solo lo transporta', de: 'Der Server transportiert sie nur', it: 'Il server si limita a trasportarlo' }),
    text: L({
      pt: 'O servidor da SEALER entrega o envelope trancado. Não consegue ler o que está lá dentro, e o envelope não diz quem o enviou.',
      en: 'The SEALER server delivers the locked envelope. It cannot read what is inside, and the envelope does not say who sent it.',
      fr: 'Le serveur de SEALER remet l’enveloppe verrouillée. Il ne peut pas lire ce qu’il y a dedans, et l’enveloppe ne dit pas qui l’a envoyée.',
      es: 'El servidor de SEALER entrega el sobre cerrado. No puede leer lo que hay dentro, y el sobre no dice quién lo envió.',
      de: 'Der Server von SEALER stellt den verschlossenen Umschlag zu. Er kann nicht lesen, was darin steht, und der Umschlag verrät nicht, wer ihn gesendet hat.',
      it: 'Il server di SEALER consegna la busta chiusa. Non può leggere cosa c’è dentro, e la busta non dice chi l’ha inviata.',
    }),
    ms: 7000,
  },
  {
    title: L({ pt: 'O Rui abre com a sua chave', en: 'Rui opens it with his key', fr: 'Rui l’ouvre avec sa clé', es: 'Rui lo abre con su llave', de: 'Rui öffnet sie mit seinem Schlüssel', it: 'Rui lo apre con la sua chiave' }),
    text: L({
      pt: 'A chave está só no telemóvel do Rui. Nem a SEALER a tem.',
      en: 'The key is only on Rui’s phone. Not even SEALER has it.',
      fr: 'La clé est seulement sur le téléphone de Rui. Même SEALER ne l’a pas.',
      es: 'La llave está solo en el móvil de Rui. Ni siquiera SEALER la tiene.',
      de: 'Der Schlüssel ist nur auf Ruis Handy. Nicht einmal SEALER hat ihn.',
      it: 'La chiave è solo sul telefono di Rui. Nemmeno SEALER ce l’ha.',
    }),
    ms: 6500,
  },
  {
    title: L({ pt: 'Protegida também para o futuro', en: 'Protected for the future too', fr: 'Protégé aussi pour l’avenir', es: 'Protegido también para el futuro', de: 'Auch für die Zukunft geschützt', it: 'Protetto anche per il futuro' }),
    text: L({
      pt: 'A SEALER usa também uma proteção pensada para os computadores quânticos, que um dia poderão quebrar a criptografia de hoje.',
      en: 'SEALER also uses protection designed for quantum computers, which may one day break today’s cryptography.',
      fr: 'SEALER utilise aussi une protection pensée pour les ordinateurs quantiques, qui pourraient un jour casser la cryptographie d’aujourd’hui.',
      es: 'SEALER también usa una protección pensada para los ordenadores cuánticos, que algún día podrían romper la criptografía de hoy.',
      de: 'SEALER nutzt auch einen Schutz, der für Quantencomputer gedacht ist, die eines Tages die heutige Kryptografie brechen könnten.',
      it: 'SEALER usa anche una protezione pensata per i computer quantistici, che un giorno potrebbero rompere la crittografia di oggi.',
    }),
    ms: 6000,
  },
];

const PLAIN = L({ pt: 'Olá, Rui!', en: 'Hi, Rui!', fr: 'Salut, Rui !', es: '¡Hola, Rui!', de: 'Hallo, Rui!', it: 'Ciao, Rui!' });
const T = {
  step: (n, m) => L({ pt: `Passo ${n} de ${m}`, en: `Step ${n} of ${m}`, fr: `Étape ${n} sur ${m}`, es: `Paso ${n} de ${m}`, de: `Schritt ${n} von ${m}`, it: `Passo ${n} di ${m}` }),
  stepLabel: (n, t) => L({ pt: `Passo ${n}: ${t}`, en: `Step ${n}: ${t}`, fr: `Étape ${n} : ${t}`, es: `Paso ${n}: ${t}`, de: `Schritt ${n}: ${t}`, it: `Passo ${n}: ${t}` }),
  pause: L({ pt: 'Pausar', en: 'Pause', fr: 'Pause', es: 'Pausa', de: 'Pause', it: 'Pausa' }),
  play: L({ pt: 'Reproduzir', en: 'Play', fr: 'Lecture', es: 'Reproducir', de: 'Abspielen', it: 'Riproduci' }),
  locked: L({ pt: 'Trancada', en: 'Locked', fr: 'Verrouillé', es: 'Cerrado', de: 'Verschlossen', it: 'Chiuso' }),
  opened: L({ pt: 'Aberta', en: 'Opened', fr: 'Ouvert', es: 'Abierto', de: 'Geöffnet', it: 'Aperto' }),
};
const CIPHER = '7Fq#9zK@';
const FIELD = L({ pt: 'Mensagem', en: 'Message', fr: 'Message', es: 'Mensaje', de: 'Nachricht', it: 'Messaggio' });
const GLYPHS = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghkmnpqrstuvwxyz0123456789#%&@$';

/** Disposições do desenho. s é a escala do envelope e da chave. */
const LAYOUTS = {
  wide: {
    viewBox: '0 0 800 450',
    s: 1,
    route: 'M210 245 H 590',
    groups: {},
    pos: { ana: [125, 245], server: [400, 245], rui: [675, 245], key: [675, 300] },
  },
  narrow: {
    viewBox: '0 0 400 900',
    s: 0.8,
    route: 'M200 262 V 326 M200 470 V 596',
    groups: {
      'g-ana': 'translate(100 -46) scale(0.8)',
      'g-server': 'translate(-200 290)',
      'g-sees': 'translate(-200 220)',
      'g-rui': 'translate(-340 544) scale(0.8)',
      'key-label': 'translate(-340 544) scale(0.8)',
      'g-quantum': 'translate(200 520) scale(0.6) translate(-400 -310)',
    },
    pos: { ana: [200, 150], server: [200, 505], rui: [200, 740], key: [200, 784] },
  },
};

const $ = (id) => document.getElementById(id);
const svg = $('scene');
const env = $('env');
const envText = $('env-text');
const envLock = $('env-lock');
const envTag = $('env-tag');
const anaText = $('ana-text');
const anaTick = $('ana-tick');
const anaMsg = $('ana-msg');
const anaField = $('ana-field');
const rkey = $('rkey');
const progress = $('progress');
const playBtn = $('play');

const reducedQuery = matchMedia('(prefers-reduced-motion: reduce)');
const narrowQuery = matchMedia('(max-width: 560px)');
const reduced = () => reducedQuery.matches;

let layout = LAYOUTS.wide;
let current = 0;
let playing = !reduced();
let timer = 0;
let run = 0; // cada passo novo invalida as animações do anterior
let started = 0;
let remaining = 0;

// ---------- Disposição ----------

function applyLayout() {
  layout = narrowQuery.matches ? LAYOUTS.narrow : LAYOUTS.wide;
  svg.setAttribute('viewBox', layout.viewBox);
  svg.dataset.layout = narrowQuery.matches ? 'narrow' : 'wide';
  $('route').setAttribute('d', layout.route);
  for (const id of Object.keys(LAYOUTS.narrow.groups)) {
    const t = layout.groups[id];
    if (t) $(id).setAttribute('transform', t);
    else $(id).removeAttribute('transform');
  }
}

// ---------- Ajudas de animação ----------

const wait = (ms) => new Promise((resolve) => setTimeout(resolve, reduced() ? 0 : ms));
const alive = (id) => id === run;
const place = (el, [x, y]) => el.setAttribute('transform', `translate(${x} ${y}) scale(${layout.s})`);

function tween(el, from, to, ms, id) {
  if (reduced()) {
    place(el, to);
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    const t0 = performance.now();
    const ease = (t) => (t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2);
    function frame(now) {
      if (!alive(id)) return resolve();
      const t = Math.min(1, Math.max(0, (now - t0) / ms));
      const e = ease(t);
      place(el, [from[0] + (to[0] - from[0]) * e, from[1] + (to[1] - from[1]) * e]);
      if (t < 1) requestAnimationFrame(frame);
      else resolve();
    }
    requestAnimationFrame(frame);
  });
}

/** Troca o texto do envelope letra a letra, com caracteres ao acaso pelo meio. */
function scramble(from, to, ms, id) {
  if (reduced()) {
    envText.textContent = to;
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    const len = Math.max(from.length, to.length);
    const t0 = performance.now();
    function frame(now) {
      if (!alive(id)) return resolve();
      const t = Math.min(1, Math.max(0, (now - t0) / ms));
      let out = '';
      for (let i = 0; i < len; i++) {
        const settle = (i + 1) / len;
        if (t >= settle) out += to[i] ?? '';
        else if (t > settle - 0.45) out += GLYPHS[(Math.random() * GLYPHS.length) | 0];
        else out += from[i] ?? '';
      }
      envText.textContent = out;
      if (t < 1) requestAnimationFrame(frame);
      else resolve();
    }
    requestAnimationFrame(frame);
  });
}

/** A Ana escreve na barra da app; ao enviar, a mensagem aparece na conversa. */
async function type(text, id) {
  anaField.classList.add('typing');
  if (!reduced()) {
    for (let i = 0; i <= text.length; i++) {
      if (!alive(id)) return;
      anaField.textContent = text.slice(0, i);
      await wait(130);
    }
    await wait(450);
    if (!alive(id)) return;
  }
  resetField();
  anaMsg.classList.remove('hidden');
}

function resetField() {
  anaField.classList.remove('typing');
  anaField.textContent = FIELD;
}

function tag(text, kind) {
  envTag.textContent = text;
  envTag.setAttribute('class', kind ? `tag tag--${kind}` : 'tag');
}

function setEnv(visible, locked, text, pos) {
  env.setAttribute('opacity', visible ? '1' : '0');
  envLock.setAttribute('opacity', locked ? '1' : '0');
  envLock.classList.remove('open');
  envText.textContent = text;
  tag('');
  place(env, pos);
}

// ---------- Passos ----------

async function animate(n, id) {
  const P = layout.pos;
  anaText.textContent = PLAIN;
  anaTick.setAttribute('opacity', n >= 2 ? '1' : '0');
  svg.classList.toggle('got', n >= 5);
  svg.classList.remove('used');
  resetField();
  anaMsg.classList.toggle('hidden', n === 1);
  place(rkey, P.key);

  if (n === 1) {
    setEnv(false, false, PLAIN, P.ana);
    await wait(500);
    await type(PLAIN, id);
  } else if (n === 2) {
    setEnv(true, false, PLAIN, P.ana);
    await wait(1000);
    if (!alive(id)) return;
    await scramble(PLAIN, CIPHER, 1600, id);
    if (!alive(id)) return;
    envLock.setAttribute('opacity', '1');
    tag(T.locked, 'key');
  } else if (n === 3) {
    setEnv(true, true, CIPHER, P.ana);
    await wait(500);
    await tween(env, P.ana, P.server, 1800, id);
    await wait(1800);
    if (!alive(id)) return;
    await tween(env, P.server, P.rui, 1800, id);
  } else if (n === 4) {
    setEnv(true, true, CIPHER, P.rui);
    await wait(800);
    // A chave do Rui sobe até ao cadeado e abre-o.
    await tween(rkey, P.key, [P.rui[0] + 4 * layout.s, P.rui[1] - 34 * layout.s], 1100, id);
    if (!alive(id)) return;
    envLock.classList.add('open');
    await wait(500);
    if (!alive(id)) return;
    envLock.setAttribute('opacity', '0');
    svg.classList.add('used');
    place(rkey, P.key);
    await scramble(CIPHER, PLAIN, 1300, id);
    if (!alive(id)) return;
    tag(T.opened, 'ok');
    svg.classList.add('got');
    await wait(900);
    if (alive(id)) env.setAttribute('opacity', '0');
  } else {
    setEnv(false, false, PLAIN, P.rui);
  }
}

// ---------- Navegação ----------

const dots = SCENES.map((scene, i) => {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = 'cf-dot';
  b.setAttribute('aria-label', T.stepLabel(i + 1, scene.title));
  b.addEventListener('click', () => {
    setPlaying(false);
    go(i);
  });
  $('dots').append(b);
  return b;
});

function restartProgress(ms) {
  progress.classList.remove('run', 'paused');
  void progress.offsetWidth; // reinicia a animação CSS da barra
  progress.style.setProperty('--dur', `${ms}ms`);
  progress.classList.add('run');
  if (!playing) progress.classList.add('paused');
}

function go(i) {
  current = (i + SCENES.length) % SCENES.length;
  const n = current + 1;
  const scene = SCENES[current];
  run += 1;
  svg.dataset.scene = String(n);
  $('cap-step').textContent = T.step(n, SCENES.length);
  $('cap-title').textContent = scene.title;
  $('cap-text').textContent = scene.text;
  dots.forEach((d, j) => (j === current ? d.setAttribute('aria-current', 'step') : d.removeAttribute('aria-current')));
  animate(n, run);
  remaining = scene.ms;
  restartProgress(scene.ms);
  schedule();
}

function schedule() {
  clearTimeout(timer);
  if (!playing) return;
  started = performance.now();
  timer = setTimeout(() => go(current + 1), remaining);
}

function setPlaying(p) {
  if (playing && !p) remaining = Math.max(0, remaining - (performance.now() - started));
  playing = p;
  playBtn.textContent = p ? T.pause : T.play;
  progress.classList.toggle('paused', !p);
  schedule();
}

playBtn.addEventListener('click', () => setPlaying(!playing));
$('prev').addEventListener('click', () => {
  setPlaying(false);
  go(current - 1);
});
$('next').addEventListener('click', () => {
  setPlaying(false);
  go(current + 1);
});
document.addEventListener('keydown', (e) => {
  if (e.target.closest?.('input, textarea, select')) return;
  if (e.key === 'ArrowRight') {
    setPlaying(false);
    go(current + 1);
  } else if (e.key === 'ArrowLeft') {
    setPlaying(false);
    go(current - 1);
  }
});

// Ao rodar o telemóvel ou mudar o tamanho da janela, redesenha o passo atual.
narrowQuery.addEventListener('change', () => {
  applyLayout();
  go(current);
});

// Separador escondido: pausa o avanço automático para não saltar passos.
document.addEventListener('visibilitychange', () => {
  if (document.hidden && playing) setPlaying(false);
});

applyLayout();
playBtn.textContent = playing ? T.pause : T.play;
anaText.textContent = PLAIN;
envText.textContent = PLAIN;
translatePage();
go(0);
