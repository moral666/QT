//! Tempo de entrega das mensagens, ANONIMO: desde que o relay recebe uma
//! mensagem (SendMessage) ate o destinatario confirmar que a recebeu
//! (AckMessages).
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
//! `https://<relay>/estado.json` e que o site sealer.pt mostra).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Espera maxima que ainda conta para a media.
pub const MAX_WAIT: Duration = Duration::from_secs(60);
/// Janela da media, em minutos.
pub const WINDOW_MINUTES: usize = 60;
/// Limite de mensagens acompanhadas ao mesmo tempo (protege a memoria).
const MAX_TRACKED: usize = 50_000;

/// Resultado da janela atual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// Media em ms; `None` se nao houve mensagens entregues na janela.
    pub avg_ms: Option<u64>,
    /// Mensagens entregues (confirmadas) que entraram na media.
    pub messages: u64,
    /// Minutos que a janela ja tem (menos de `WINDOW_MINUTES` logo apos arrancar).
    pub minutes: usize,
}

#[derive(Default)]
pub struct Tracker {
    pending: HashMap<u64, Instant>,
    minute_sum_ms: u64,
    minute_count: u64,
    /// (soma de ms, mensagens) de cada minuto fechado, do mais antigo ao mais recente.
    window: VecDeque<(u64, u64)>,
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
                    self.minute_sum_ms += waited.as_millis() as u64;
                    self.minute_count += 1;
                }
            }
        }
    }

    /// Fecha o minuto atual, esquece as mensagens a espera ha mais de `MAX_WAIT`
    /// e devolve o resumo da janela.
    pub fn roll_minute(&mut self, now: Instant) -> Summary {
        self.window.push_back((self.minute_sum_ms, self.minute_count));
        self.minute_sum_ms = 0;
        self.minute_count = 0;
        while self.window.len() > WINDOW_MINUTES {
            self.window.pop_front();
        }
        self.pending.retain(|_, at| now.saturating_duration_since(*at) <= MAX_WAIT);
        self.summary()
    }

    pub fn summary(&self) -> Summary {
        let (sum, n) = self.window.iter().fold((0u64, 0u64), |acc, m| (acc.0 + m.0, acc.1 + m.1));
        // Media arredondada ao ms; sem mensagens, `None`.
        let avg_ms = (sum + n / 2).checked_div(n);
        Summary { avg_ms, messages: n, minutes: self.window.len() }
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

/// Chamado uma vez por minuto pelo `relay_server`.
pub fn roll_minute() -> Summary {
    with(|t| t.roll_minute(Instant::now()))
}

/// Ultima media conhecida (a mais recente janela com mensagens). Fica no ficheiro
/// de estado e e lida de novo ao arrancar, para o site nunca ficar sem numero
/// (sem mensagens na ultima hora, ou depois de reiniciar o relay).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastReading {
    pub avg_ms: u64,
    /// Quando foi medida (segundos desde 1970).
    pub at_unix: u64,
}

/// Le a ultima medicao de um ficheiro de estado escrito antes (por esta versao,
/// ou pela anterior, que so tinha `entrega_media_ms` e `atualizado`).
pub fn parse_last(json: &str) -> Option<LastReading> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let pair = |a: &str, t: &str| Some(LastReading { avg_ms: v.get(a)?.as_u64()?, at_unix: v.get(t)?.as_u64()? });
    pair("ultima_media_ms", "ultima_medicao").or_else(|| pair("entrega_media_ms", "atualizado"))
}

/// JSON publicado em `RELAY_STATUS_FILE` (sem serde na escrita: so numeros).
pub fn to_json(s: &Summary, updated_unix: u64, last: Option<LastReading>) -> String {
    let opt = |v: Option<u64>| v.map(|v| v.to_string()).unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"entrega_media_ms\":{},\"mensagens\":{},\"janela_minutos\":{},\"espera_maxima_s\":{},\"atualizado\":{updated_unix},\"ultima_media_ms\":{},\"ultima_medicao\":{}}}\n",
        opt(s.avg_ms),
        s.messages,
        s.minutes,
        MAX_WAIT.as_secs(),
        opt(last.map(|l| l.avg_ms)),
        opt(last.map(|l| l.at_unix)),
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
        assert_eq!(s, Summary { avg_ms: Some(201), messages: 2, minutes: 1 });
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
        assert_eq!(s, Summary { avg_ms: None, messages: 0, minutes: 1 });
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
    fn json_do_estado() {
        let s = Summary { avg_ms: Some(1520), messages: 37, minutes: 60 };
        let last = Some(LastReading { avg_ms: 1520, at_unix: 1_792_000_000 });
        assert_eq!(
            to_json(&s, 1_792_000_000, last),
            "{\"entrega_media_ms\":1520,\"mensagens\":37,\"janela_minutos\":60,\"espera_maxima_s\":60,\"atualizado\":1792000000,\"ultima_media_ms\":1520,\"ultima_medicao\":1792000000}\n"
        );
        let vazio = Summary { avg_ms: None, messages: 0, minutes: 3 };
        let j = to_json(&vazio, 1, None);
        assert!(j.starts_with("{\"entrega_media_ms\":null,") && j.contains("\"ultima_media_ms\":null,\"ultima_medicao\":null}"), "{j}");
    }

    #[test]
    fn ultima_medicao_sobrevive_a_um_reinicio() {
        // Ficheiro desta versao: a ultima medicao vem dos campos proprios.
        let last = LastReading { avg_ms: 1311, at_unix: 1_792_100_000 };
        let j = to_json(&Summary { avg_ms: None, messages: 0, minutes: 60 }, 1_792_200_000, Some(last));
        assert_eq!(parse_last(&j), Some(last));
        // Ficheiro da versao anterior: usa a media e a hora em que foi escrito.
        let antigo = "{\"entrega_media_ms\":2096,\"mensagens\":9,\"janela_minutos\":12,\"espera_maxima_s\":60,\"atualizado\":1792000000}";
        assert_eq!(parse_last(antigo), Some(LastReading { avg_ms: 2096, at_unix: 1_792_000_000 }));
        // Sem medicao nenhuma, ou ficheiro estragado: nada.
        assert_eq!(parse_last("{\"entrega_media_ms\":null,\"atualizado\":1}"), None);
        assert_eq!(parse_last("lixo"), None);
    }
}
