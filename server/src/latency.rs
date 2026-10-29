//! Tempos das mensagens, ANONIMOS. Dois numeros:
//! - envio (ida): do telemovel de quem envia ate ao relay. Logo depois de aceitar
//!   uma mensagem, o relay manda um ping nessa ligacao e mede a ida e volta ate ao
//!   pong; a ida e metade (ver `connection.rs`, `IdaProbe`). E o numero do site.
//! - entrega: desde que o relay recebe uma mensagem (SendMessage) ate o
//!   destinatario confirmar que a recebeu (AckMessages). So conta quando o
//!   destinatario estava ligado com a app aberta nesse instante
//!   (`waiters::is_online`): com a app fechada ou em segundo plano o numero mediria
//!   quanto tempo a pessoa demorou a abrir a app, e nao a entrega.
//!
//! So em memoria. Para cada mensagem em fila guarda-se o id interno da fila e o
//! instante em que chegou, e nada mais: nem remetente, nem destinatario, nem
//! tamanho. A entrada desaparece quando a mensagem e confirmada ou ao fim de
//! `MAX_WAIT`. Mensagens que esperam mais do que isso (destinatario offline)
//! nao contam para a media: mediriam a ausencia da pessoa, nao o servico.
//! Ao reiniciar o relay, perde-se tudo.
//!
//! O que sai daqui: a media (em ms) e o numero de mensagens dos ultimos
//! `WINDOW_MINUTES` minutos, e a ultima media conhecida (`LastReading`), que
//! fica no ficheiro de estado para o site nunca ficar sem numero. O `relay_server` escreve-o uma vez por minuto em
//! `RELAY_STATUS_FILE` (um JSON pequeno que o Caddy publica em
//! `https://<relay>/estado.json` e que o site sealerapp.com mostra).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Espera maxima que ainda conta para a media.
pub const MAX_WAIT: Duration = Duration::from_secs(60);
/// Janela da media, em minutos.
pub const WINDOW_MINUTES: usize = 60;
/// Limite de mensagens acompanhadas ao mesmo tempo (protege a memoria).
const MAX_TRACKED: usize = 50_000;
/// Ida e volta maxima que ainda conta (acima disto, a ligacao estava presa).
pub const MAX_ROUND_TRIP: Duration = Duration::from_secs(10);
/// Tempos guardados por minuto para a mediana e o p95 (protege a memoria; a media conta todos).
const MAX_SAMPLES_PER_MINUTE: usize = 2_000;

/// Resultado da janela atual.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Summary {
    /// Media em ms; `None` se nao houve mensagens entregues na janela.
    pub avg_ms: Option<u64>,
    /// Mediana em ms (o tempo da mensagem tipica): uma mensagem para quem nao estava
    /// a olhar para a app (dezenas de segundos) quase nao a mexe, ao contrario da media.
    pub median_ms: Option<u64>,
    /// 95 % das mensagens chegaram em menos do que isto.
    pub p95_ms: Option<u64>,
    /// Mensagens entregues (confirmadas) que entraram na media.
    pub messages: u64,
    /// Minutos que a janela ja tem (menos de `WINDOW_MINUTES` logo apos arrancar).
    pub minutes: usize,
}

#[derive(Default, Clone)]
struct Minute {
    sum_ms: u64,
    count: u64,
    samples: Vec<u32>,
}

/// Tempos em ms agrupados por minuto, com os ultimos `WINDOW_MINUTES` minutos.
#[derive(Default)]
struct Window {
    minute: Minute,
    /// Minutos fechados, do mais antigo ao mais recente.
    closed: VecDeque<Minute>,
}

impl Window {
    fn push(&mut self, ms: u64) {
        self.minute.sum_ms += ms;
        self.minute.count += 1;
        if self.minute.samples.len() < MAX_SAMPLES_PER_MINUTE {
            self.minute.samples.push(ms.min(u32::MAX as u64) as u32);
        }
    }

    fn roll(&mut self) {
        self.closed.push_back(std::mem::take(&mut self.minute));
        while self.closed.len() > WINDOW_MINUTES {
            self.closed.pop_front();
        }
    }

    fn summary(&self) -> Summary {
        let (sum, n) = self.closed.iter().fold((0u64, 0u64), |acc, m| (acc.0 + m.sum_ms, acc.1 + m.count));
        // Media arredondada ao ms; sem amostras, `None`.
        let avg_ms = (sum + n / 2).checked_div(n);
        let mut all: Vec<u32> = self.closed.iter().flat_map(|m| m.samples.iter().copied()).collect();
        all.sort_unstable();
        let pct = |p: f64| -> Option<u64> {
            if all.is_empty() {
                return None;
            }
            let i = ((all.len() - 1) as f64 * p).round() as usize;
            Some(all[i] as u64)
        };
        Summary { avg_ms, median_ms: pct(0.5), p95_ms: pct(0.95), messages: n, minutes: self.closed.len() }
    }
}

#[derive(Default)]
pub struct Tracker {
    pending: HashMap<u64, Instant>,
    /// Entrega: do relay ate o destinatario confirmar.
    delivery: Window,
    /// Ida: do telemovel de quem envia ate ao relay (metade da ida e volta, ver `ida_sample`).
    ida: Window,
}

impl Tracker {
    /// Uma mensagem entrou na fila.
    pub fn queued(&mut self, id: u64, at: Instant) {
        if self.pending.len() < MAX_TRACKED {
            self.pending.insert(id, at);
        }
    }

    /// O destinatario confirmou estas mensagens (so ids que sairam mesmo da fila).
    pub fn acked(&mut self, ids: &[u64], now: Instant) {
        for id in ids {
            if let Some(at) = self.pending.remove(id) {
                let waited = now.saturating_duration_since(at);
                if waited <= MAX_WAIT {
                    self.delivery.push(waited.as_millis() as u64);
                }
            }
        }
    }

    /// Uma ida e volta (ping do relay -> pong do telemovel) logo a seguir a uma
    /// mensagem enviada. Conta metade: o tempo de ida do telemovel ate ao relay.
    pub fn ida_sample(&mut self, round_trip: Duration) {
        if round_trip <= MAX_ROUND_TRIP {
            self.ida.push((round_trip.as_micros() as u64 / 2 + 500) / 1000);
        }
    }

    /// Fecha o minuto atual, esquece as mensagens a espera ha mais de `MAX_WAIT`
    /// e devolve o resumo da janela (entrega).
    pub fn roll_minute(&mut self, now: Instant) -> Summary {
        crate::waiters::prune_recent();
        self.delivery.roll();
        self.ida.roll();
        self.pending.retain(|_, at| now.saturating_duration_since(*at) <= MAX_WAIT);
        self.summary()
    }

    /// Resumo da entrega (relay -> destinatario).
    pub fn summary(&self) -> Summary {
        self.delivery.summary()
    }

    /// Resumo da ida (telemovel de quem envia -> relay).
    pub fn ida_summary(&self) -> Summary {
        self.ida.summary()
    }

    pub fn tracked(&self) -> usize {
        self.pending.len()
    }
}

static TRACKER: Mutex<Option<Tracker>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Tracker) -> R) -> R {
    let mut guard = TRACKER.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Tracker::default))
}

/// Chamado pelo relay quando uma mensagem entra na fila.
pub fn queued(id: u64) {
    with(|t| t.queued(id, Instant::now()));
}

/// Chamado pelo relay com os ids que uma confirmacao tirou da fila.
pub fn acked(ids: &[u64]) {
    if ids.is_empty() {
        return;
    }
    with(|t| t.acked(ids, Instant::now()));
}

/// Chamado pelo relay quando chega o pong de um ping enviado logo a seguir a uma mensagem.
pub fn ida_sample(round_trip: Duration) {
    with(|t| t.ida_sample(round_trip));
}

/// Chamado uma vez por minuto pelo `relay_server`: fecha o minuto e devolve
/// os resumos da entrega e da ida.
pub fn roll_minute() -> (Summary, Summary) {
    with(|t| {
        let entrega = t.roll_minute(Instant::now());
        (entrega, t.ida_summary())
    })
}

/// Ultima media conhecida (a mais recente janela com mensagens). Fica no ficheiro
/// de estado e e lida de novo ao arrancar, para o site nunca ficar sem numero
/// (sem mensagens na ultima hora, ou depois de reiniciar o relay).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastReading {
    pub avg_ms: u64,
    /// Mediana da mesma janela (o numero que o site mostra); `None` em ficheiros antigos.
    pub median_ms: Option<u64>,
    /// Quando foi medida (segundos desde 1970).
    pub at_unix: u64,
}

/// Le a ultima medicao de um ficheiro de estado escrito antes (por esta versao,
/// ou pela anterior, que so tinha `entrega_media_ms` e `atualizado`).
pub fn parse_last(json: &str) -> Option<LastReading> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let med = v.get("ultima_mediana_ms").and_then(|m| m.as_u64());
    let pair = |a: &str, t: &str, m: Option<u64>| {
        Some(LastReading { avg_ms: v.get(a)?.as_u64()?, median_ms: m, at_unix: v.get(t)?.as_u64()? })
    };
    pair("ultima_media_ms", "ultima_medicao", med).or_else(|| pair("entrega_media_ms", "atualizado", None))
}

/// Como `parse_last`, para a ultima media da ida (telemovel -> relay).
pub fn parse_last_ida(json: &str) -> Option<LastReading> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    Some(LastReading {
        avg_ms: v.get("ultima_ida_ms")?.as_u64()?,
        median_ms: v.get("ultima_ida_mediana_ms").and_then(|m| m.as_u64()),
        at_unix: v.get("ultima_ida_medicao")?.as_u64()?,
    })
}

/// JSON publicado em `RELAY_STATUS_FILE` (sem serde na escrita: so numeros).
pub fn to_json(s: &Summary, updated_unix: u64, last: Option<LastReading>, ida: &Summary, last_ida: Option<LastReading>) -> String {
    let opt = |v: Option<u64>| v.map(|v| v.to_string()).unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"entrega_media_ms\":{},\"entrega_mediana_ms\":{},\"entrega_p95_ms\":{},\"mensagens\":{},\"janela_minutos\":{},\"espera_maxima_s\":{},\"atualizado\":{updated_unix},\"ultima_media_ms\":{},\"ultima_mediana_ms\":{},\"ultima_medicao\":{},\"ida_media_ms\":{},\"ida_mediana_ms\":{},\"ida_p95_ms\":{},\"ida_amostras\":{},\"ultima_ida_ms\":{},\"ultima_ida_mediana_ms\":{},\"ultima_ida_medicao\":{}}}\n",
        opt(s.avg_ms),
        opt(s.median_ms),
        opt(s.p95_ms),
        s.messages,
        s.minutes,
        MAX_WAIT.as_secs(),
        opt(last.map(|l| l.avg_ms)),
        opt(last.and_then(|l| l.median_ms)),
        opt(last.map(|l| l.at_unix)),
        opt(ida.avg_ms),
        opt(ida.median_ms),
        opt(ida.p95_ms),
        ida.messages,
        opt(last_ida.map(|l| l.avg_ms)),
        opt(last_ida.and_then(|l| l.median_ms)),
        opt(last_ida.map(|l| l.at_unix)),
    )
}

/// Escreve o ficheiro de estado de forma atomica (escreve ao lado e renomeia),
/// legivel por todos (0644) para o Caddy o poder servir.
pub fn write_status_file(path: &str, json: &str) -> std::io::Result<()> {
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_das_mensagens_confirmadas() {
        let t0 = Instant::now();
        let mut t = Tracker::default();
        t.queued(1, t0);
        t.queued(2, t0);
        t.acked(&[1], t0 + Duration::from_millis(100));
        t.acked(&[2], t0 + Duration::from_millis(301));
        let s = t.roll_minute(t0 + Duration::from_secs(1));
        assert_eq!(s, Summary { avg_ms: Some(201), median_ms: Some(301), p95_ms: Some(301), messages: 2, minutes: 1 });
        assert_eq!(t.tracked(), 0);
    }

    #[test]
    fn confirmacao_repetida_ou_desconhecida_nao_conta() {
        let t0 = Instant::now();
        let mut t = Tracker::default();
        t.queued(7, t0);
        t.acked(&[7, 99], t0 + Duration::from_millis(50));
        t.acked(&[7], t0 + Duration::from_millis(5000));
        let s = t.roll_minute(t0 + Duration::from_secs(6));
        assert_eq!(s.messages, 1);
        assert_eq!(s.avg_ms, Some(50));
    }

    #[test]
    fn destinatario_offline_nao_entra_na_media() {
        let t0 = Instant::now();
        let mut t = Tracker::default();
        t.queued(1, t0);
        t.queued(2, t0);
        t.acked(&[1], t0 + MAX_WAIT + Duration::from_millis(1));
        let s = t.roll_minute(t0 + MAX_WAIT + Duration::from_secs(1));
        assert_eq!(s, Summary { avg_ms: None, median_ms: None, p95_ms: None, messages: 0, minutes: 1 });
        // A mensagem 2 (nunca confirmada) foi esquecida ao fechar o minuto.
        assert_eq!(t.tracked(), 0);
    }

    #[test]
    fn janela_guarda_so_os_ultimos_minutos() {
        let t0 = Instant::now();
        let mut t = Tracker::default();
        t.queued(1, t0);
        t.acked(&[1], t0 + Duration::from_millis(1000));
        t.roll_minute(t0);
        for _ in 0..WINDOW_MINUTES {
            t.roll_minute(t0);
        }
        let s = t.summary();
        assert_eq!(s.minutes, WINDOW_MINUTES);
        assert_eq!(s.messages, 0);
        assert_eq!(s.avg_ms, None);
    }

    #[test]
    fn mediana_nao_e_estragada_por_quem_nao_estava_a_olhar() {
        // 9 mensagens a 200 ms e 1 que esperou 40 s (telemovel com o ecra desligado).
        let t0 = Instant::now();
        let mut t = Tracker::default();
        for id in 0..10u64 {
            t.queued(id, t0);
            let ms = if id == 9 { 40_000 } else { 200 };
            t.acked(&[id], t0 + Duration::from_millis(ms));
        }
        let s = t.roll_minute(t0 + Duration::from_secs(41));
        assert_eq!(s.avg_ms, Some(4_180), "a media e puxada pela mensagem lenta");
        assert_eq!(s.median_ms, Some(200), "a mediana mostra a mensagem tipica");
        assert_eq!(s.p95_ms, Some(40_000));
    }

    #[test]
    fn ida_e_metade_da_ida_e_volta() {
        let t0 = Instant::now();
        let mut t = Tracker::default();
        t.ida_sample(Duration::from_millis(40));
        t.ida_sample(Duration::from_millis(61));
        t.ida_sample(MAX_ROUND_TRIP + Duration::from_millis(1)); // ligacao presa: nao conta
        t.roll_minute(t0);
        let s = t.ida_summary();
        assert_eq!(s.messages, 2);
        assert_eq!(s.avg_ms, Some(26), "20 ms e 31 ms (30,5 arredondado)");
        assert_eq!(t.summary().messages, 0, "a ida nao se mistura com a entrega");
    }

    #[test]
    fn json_do_estado() {
        let s = Summary { avg_ms: Some(1520), median_ms: Some(180), p95_ms: Some(9000), messages: 37, minutes: 60 };
        let last = Some(LastReading { avg_ms: 1520, median_ms: Some(180), at_unix: 1_792_000_000 });
        assert_eq!(
            to_json(&s, 1_792_000_000, last, &Summary::default(), None),
            "{\"entrega_media_ms\":1520,\"entrega_mediana_ms\":180,\"entrega_p95_ms\":9000,\"mensagens\":37,\"janela_minutos\":60,\"espera_maxima_s\":60,\"atualizado\":1792000000,\"ultima_media_ms\":1520,\"ultima_mediana_ms\":180,\"ultima_medicao\":1792000000,\"ida_media_ms\":null,\"ida_mediana_ms\":null,\"ida_p95_ms\":null,\"ida_amostras\":0,\"ultima_ida_ms\":null,\"ultima_ida_mediana_ms\":null,\"ultima_ida_medicao\":null}\n"
        );
        let vazio = Summary { minutes: 3, ..Summary::default() };
        let j = to_json(&vazio, 1, None, &vazio, None);
        assert!(j.starts_with("{\"entrega_media_ms\":null,") && j.contains("\"ultima_media_ms\":null,\"ultima_mediana_ms\":null,\"ultima_medicao\":null,"), "{j}");
        let ida = Summary { avg_ms: Some(23), median_ms: Some(21), p95_ms: Some(60), messages: 12, minutes: 60 };
        let j = to_json(&vazio, 1, None, &ida, Some(LastReading { avg_ms: 23, median_ms: Some(21), at_unix: 5 }));
        assert!(j.contains("\"ida_media_ms\":23,\"ida_mediana_ms\":21,\"ida_p95_ms\":60,\"ida_amostras\":12,\"ultima_ida_ms\":23,\"ultima_ida_mediana_ms\":21,\"ultima_ida_medicao\":5}"), "{j}");
        assert_eq!(parse_last_ida(&j), Some(LastReading { avg_ms: 23, median_ms: Some(21), at_unix: 5 }));
        assert_eq!(parse_last_ida("{\"entrega_media_ms\":1}"), None);
    }

    #[test]
    fn ultima_medicao_sobrevive_a_um_reinicio() {
        // Ficheiro desta versao: a ultima medicao vem dos campos proprios.
        let last = LastReading { avg_ms: 1311, median_ms: Some(150), at_unix: 1_792_100_000 };
        let j = to_json(&Summary { minutes: 60, ..Summary::default() }, 1_792_200_000, Some(last), &Summary::default(), None);
        assert_eq!(parse_last(&j), Some(last));
        // Ficheiro da versao anterior: usa a media e a hora em que foi escrito.
        let antigo = "{\"entrega_media_ms\":2096,\"mensagens\":9,\"janela_minutos\":12,\"espera_maxima_s\":60,\"atualizado\":1792000000}";
        assert_eq!(parse_last(antigo), Some(LastReading { avg_ms: 2096, median_ms: None, at_unix: 1_792_000_000 }));
        // Ficheiro da 0.9.3 (sem mediana): le a media e fica sem mediana.
        let sem_mediana = "{\"ultima_media_ms\":900,\"ultima_medicao\":7,\"ultima_ida_ms\":40,\"ultima_ida_medicao\":7}";
        assert_eq!(parse_last(sem_mediana), Some(LastReading { avg_ms: 900, median_ms: None, at_unix: 7 }));
        assert_eq!(parse_last_ida(sem_mediana), Some(LastReading { avg_ms: 40, median_ms: None, at_unix: 7 }));
        // Sem medicao nenhuma, ou ficheiro estragado: nada.
        assert_eq!(parse_last("{\"entrega_media_ms\":null,\"atualizado\":1}"), None);
        assert_eq!(parse_last("lixo"), None);
    }
}
