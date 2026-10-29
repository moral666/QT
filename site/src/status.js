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

// A app está em testes fechados (versão 0.9.x): o marcador mostra o estado do serviço usado
// pelos testadores, não quer dizer que a app já esteja disponível ao público.
// Número de testadores: alterar aqui e no index.html (status-pill__count).
const TESTING = L({
  pt: 'A SEALER está em testes fechados: por agora só 5 testadores convidados têm acesso ao serviço.',
  en: 'SEALER is in closed testing: for now only 5 invited testers have access to the service.',
  fr: 'SEALER est en test fermé : pour l’instant, seuls 5 testeurs invités ont accès au service.',
  es: 'SEALER está en pruebas cerradas: por ahora solo 5 probadores invitados tienen acceso al servicio.',
  de: 'SEALER ist in einem geschlossenen Test: Vorerst haben nur 5 eingeladene Tester Zugang zum Dienst.',
  it: 'SEALER è in test chiuso: per ora solo 5 tester invitati hanno accesso al servizio.',
  ja: 'SEALERはクローズドテスト中です。現在、サービスを利用できるのは招待された5人のテスターだけです。',
  zh: 'SEALER正在封闭测试中：目前只有5名受邀测试人员可以使用该服务。',
  hi: 'SEALER क्लोज़्ड टेस्टिंग में है: अभी सिर्फ़ 5 आमंत्रित टेस्टर ही सेवा का इस्तेमाल कर सकते हैं।',
  ar: 'تطبيق SEALER في مرحلة اختبار مغلق: حاليًا لا يصل إلى الخدمة إلا المختبرون المدعوون (العدد: 5).',
});
const TAG = L({
  pt: 'Testes fechados<span class="status-pill__count"> · 5 pessoas</span>',
  en: 'Closed testing<span class="status-pill__count"> · 5 people</span>',
  fr: 'Test fermé<span class="status-pill__count"> · 5 personnes</span>',
  es: 'Pruebas cerradas<span class="status-pill__count"> · 5 personas</span>',
  de: 'Geschlossener Test<span class="status-pill__count"> · 5 Personen</span>',
  it: 'Test chiuso<span class="status-pill__count"> · 5 persone</span>',
  ja: 'クローズドテスト<span class="status-pill__count"> · 5人</span>',
  zh: '封闭测试<span class="status-pill__count"> · 5人</span>',
  hi: 'क्लोज़्ड टेस्टिंग<span class="status-pill__count"> · 5 लोग</span>',
  ar: 'اختبار مغلق<span class="status-pill__count"> · المختبرون: 5</span>',
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
 * "Há n minutos/horas/dias" em árabe: a palavra muda com o número (2 tem forma própria,
 * 3 a 10 usa o plural, 11 ou mais o singular). n nunca é 1 aqui (há min1, h1).
 */
function arAgo(n, two, few, many) {
  if (n === 2) return `منذ ${two}`;
  const r = n % 100;
  return `منذ ${n} ${r >= 3 && r <= 10 ? few : many}`;
}

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
  fr: {
    how: 'le temps que met un message type depuis le téléphone de l’expéditeur jusqu’à ce que le téléphone du destinataire confirme qu’il l’a (envoi au serveur plus remise)',
    now: 'à l’instant', min1: 'il y a 1 minute', mins: (n) => `il y a ${n} minutes`, h1: 'il y a 1 heure', hs: (n) => `il y a ${n} heures`, days: (n) => `il y a ${n} jours`,
    msg1: '1 message', msgs: (n) => `${n} messages`,
    hour: (v, n, how) => ` Temps entre téléphones dans la dernière heure : ${v} ms (${n}), ${how}.`,
    last: (v, when, how) => ` Dernier temps entre téléphones : ${v} ms${when}, ${how}.`,
    measured: (a) => ` (mesuré ${a})`,
  },
  de: {
    how: 'die Zeit, die eine typische Nachricht vom Handy des Absenders braucht, bis das Handy des Empfängers bestätigt, dass sie angekommen ist (Senden an den Server plus Zustellung)',
    now: 'gerade eben', min1: 'vor 1 Minute', mins: (n) => `vor ${n} Minuten`, h1: 'vor 1 Stunde', hs: (n) => `vor ${n} Stunden`, days: (n) => `vor ${n} Tagen`,
    msg1: '1 Nachricht', msgs: (n) => `${n} Nachrichten`,
    hour: (v, n, how) => ` Zeit zwischen Handys in der letzten Stunde: ${v} ms (${n}), ${how}.`,
    last: (v, when, how) => ` Letzte Zeit zwischen Handys: ${v} ms${when}, ${how}.`,
    measured: (a) => ` (gemessen ${a})`,
  },
  it: {
    how: 'il tempo che un messaggio tipico impiega dal telefono di chi lo invia finché il telefono di chi lo riceve conferma di averlo (invio al server più consegna)',
    now: 'proprio ora', min1: '1 minuto fa', mins: (n) => `${n} minuti fa`, h1: '1 ora fa', hs: (n) => `${n} ore fa`, days: (n) => `${n} giorni fa`,
    msg1: '1 messaggio', msgs: (n) => `${n} messaggi`,
    hour: (v, n, how) => ` Tempo tra telefoni nell’ultima ora: ${v} ms (${n}), ${how}.`,
    last: (v, when, how) => ` Ultimo tempo tra telefoni: ${v} ms${when}, ${how}.`,
    measured: (a) => ` (misurato ${a})`,
  },
  es: {
    how: 'el tiempo que tarda un mensaje típico desde el móvil de quien lo envía hasta que el móvil de quien lo recibe confirma que lo tiene (envío al servidor más entrega)',
    now: 'ahora mismo', min1: 'hace 1 minuto', mins: (n) => `hace ${n} minutos`, h1: 'hace 1 hora', hs: (n) => `hace ${n} horas`, days: (n) => `hace ${n} días`,
    msg1: '1 mensaje', msgs: (n) => `${n} mensajes`,
    hour: (v, n, how) => ` Tiempo entre móviles en la última hora: ${v} ms (${n}), ${how}.`,
    last: (v, when, how) => ` Último tiempo entre móviles: ${v} ms${when}, ${how}.`,
    measured: (a) => ` (medido ${a})`,
  },
  ja: {
    how: '一般的なメッセージが、送る人のスマートフォンを出てから、受け取る人のスマートフォンが受信を確認するまでにかかる時間（サーバーへの送信と配信の合計）',
    now: 'たった今', min1: '1分前', mins: (n) => `${n}分前`, h1: '1時間前', hs: (n) => `${n}時間前`, days: (n) => `${n}日前`,
    msg1: '1件のメッセージ', msgs: (n) => `${n}件のメッセージ`,
    hour: (v, n, how) => `直近1時間のスマートフォン間の時間：${v} ms（${n}）。これは${how}です。`,
    last: (v, when, how) => `最後に測定したスマートフォン間の時間：${v} ms${when}。これは${how}です。`,
    measured: (a) => `（測定：${a}）`,
    stop: '。', sep: '',
  },
  zh: {
    how: '一条典型消息从发送者的手机发出，到接收者的手机确认收到所需的时间（发送到服务器加上送达）',
    now: '刚刚', min1: '1分钟前', mins: (n) => `${n}分钟前`, h1: '1小时前', hs: (n) => `${n}小时前`, days: (n) => `${n}天前`,
    msg1: '1条消息', msgs: (n) => `${n}条消息`,
    hour: (v, n, how) => `最近1小时手机到手机的耗时：${v} ms（${n}），即${how}。`,
    last: (v, when, how) => `最近一次测得的手机到手机耗时：${v} ms${when}，即${how}。`,
    measured: (a) => `（${a}测得）`,
    stop: '。', sep: '',
  },
  hi: {
    how: 'एक आम मैसेज को भेजने वाले के फ़ोन से लेकर पाने वाले के फ़ोन द्वारा उसके मिलने की पुष्टि तक लगने वाला समय (सर्वर तक भेजना और फिर पहुंचाना)',
    now: 'अभी-अभी', min1: '1 मिनट पहले', mins: (n) => `${n} मिनट पहले`, h1: '1 घंटा पहले', hs: (n) => `${n} घंटे पहले`, days: (n) => `${n} दिन पहले`,
    msg1: '1 मैसेज', msgs: (n) => `${n} मैसेज`,
    hour: (v, n, how) => ` पिछले एक घंटे में फ़ोन से फ़ोन तक का समय: ${v} ms (${n}), यानी ${how}।`,
    last: (v, when, how) => ` फ़ोन से फ़ोन तक का पिछला समय: ${v} ms${when}, यानी ${how}।`,
    measured: (a) => ` (${a} मापा गया)`,
    stop: '।',
  },
  ar: {
    how: 'وهو الوقت الذي تستغرقه رسالة نموذجية من هاتف المرسل حتى يؤكد هاتف المستلم وصولها (الإرسال إلى الخادم ثم التسليم)',
    now: 'الآن', min1: 'منذ دقيقة', mins: (n) => arAgo(n, 'دقيقتين', 'دقائق', 'دقيقة'), h1: 'منذ ساعة', hs: (n) => arAgo(n, 'ساعتين', 'ساعات', 'ساعة'), days: (n) => arAgo(n, 'يومين', 'أيام', 'يومًا'),
    msg1: 'رسالة واحدة', msgs: (n) => `الرسائل: ${n}`,
    hour: (v, n, how) => ` الوقت بين الهواتف خلال الساعة الأخيرة: \u2066${v} ms\u2069 (${n})، ${how}.`,
    last: (v, when, how) => ` آخر وقت مُقاس بين الهواتف: \u2066${v} ms\u2069${when}، ${how}.`,
    measured: (a) => ` (قيس ${a})`,
  },
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
        // Isolado: numa página em árabe fica "250 ms" e não "ms 250".
        ms.textContent = `\u2066${fmt.format(value)} ms\u2069`;
      }
    }
    const relays = Array.isArray(data?.relays) ? data.relays.filter((r) => r && typeof r.nome === 'string') : [];
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
          return item;
        }),
      );
      relaysEl.hidden = relays.length === 0;
    }
    // "Suécia: Online. Espanha: Online." (na frase completa, para leitores de ecrã e title).
    const relaysText = relays
      .map((r) => `${relayName(r.nome)}: ${TEXT[r.estado === 'on' ? 'on' : 'off'].mini}${W.stop ?? '.'}`)
      .join(W.sep ?? ' ');
    // Japones e chines: ponto final 。 e sem espacos entre frases; hindi: ponto final ।.
    const full = `${TEXT[state].long}${W.stop ?? '.'}${relaysText ? (W.sep ?? ' ') + relaysText : ''}${totalText(data)}${W.sep ?? ' '}${TESTING}`;
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
