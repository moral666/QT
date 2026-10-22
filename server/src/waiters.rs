//! Entrega imediata: acordar quem esta a espera de mensagens (`WaitMessages`).
//!
//! Um cliente com a app aberta pede `WaitMessages`: se ja houver mensagens na
//! fila, recebe-as logo; se nao, a resposta fica pendente ate chegar uma
//! mensagem para ele (ou ate `MAX_WAIT_SECS`). Assim a mensagem chega ao
//! telemovel no instante em que entra no relay, em vez de esperar pela proxima
//! verificacao periodica (antes, de 3 em 3 segundos).
//!
//! Aqui so ha um mapa `user_id -> Notify`, em memoria, com entradas apenas
//! enquanto alguem desse utilizador esta a espera. O relay ja conhece o
//! `user_id` de cada ligacao autenticada: nada de novo e guardado. Funciona com
//! um so processo do relay; com varios, isto passaria por Redis pub/sub.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

/// Espera maxima de um `WaitMessages`, em segundos. Abaixo dos tempos de
/// inatividade tipicos de proxies e operadoras moveis (30 s ou mais).
pub const MAX_WAIT_SECS: u64 = 25;

static WAITERS: Mutex<Option<HashMap<String, Arc<Notify>>>> = Mutex::new(None);

/// Margem depois de uma espera acabar em que o utilizador ainda conta como
/// ligado: entre duas esperas a app recebe, guarda e confirma (dezenas de ms).
pub const ONLINE_GRACE: Duration = Duration::from_secs(5);

/// Quando acabou a ultima espera de cada utilizador (so os ultimos `ONLINE_GRACE`;
/// o resto e apagado). Serve so para medir a entrega com o destinatario ligado.
static RECENT: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

fn with_recent<R>(f: impl FnOnce(&mut HashMap<String, Instant>) -> R) -> R {
    let mut guard = RECENT.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// Esquece as esperas que acabaram ha mais de `ONLINE_GRACE`.
fn prune(m: &mut HashMap<String, Instant>, now: Instant) {
    m.retain(|_, at| now.saturating_duration_since(*at) <= ONLINE_GRACE);
}

fn with<R>(f: impl FnOnce(&mut HashMap<String, Arc<Notify>>) -> R) -> R {
    let mut guard = WAITERS.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// Registo de espera de um utilizador; ao ser largado, limpa a entrada do mapa
/// se mais ninguem desse utilizador estiver a espera.
pub struct Waiter {
    user_id: String,
    notify: Arc<Notify>,
}

impl Waiter {
    pub fn register(user_id: &str) -> Self {
        let notify = with(|m| m.entry(user_id.to_string()).or_insert_with(|| Arc::new(Notify::new())).clone());
        Waiter { user_id: user_id.to_string(), notify }
    }

    pub fn notify(&self) -> &Notify {
        &self.notify
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        let now = Instant::now();
        with_recent(|m| {
            if m.len() >= 10_000 {
                prune(m, now);
            }
            m.insert(self.user_id.clone(), now);
        });
        with(|m| {
            // 2 = a entrada do mapa + este Waiter: ninguem mais esta a espera.
            if m.get(&self.user_id).is_some_and(|n| Arc::ptr_eq(n, &self.notify) && Arc::strong_count(n) <= 2) {
                m.remove(&self.user_id);
            }
        });
    }
}

/// Chegou uma mensagem para `user_id`: acorda quem esta a espera (se houver).
pub fn wake(user_id: &str) {
    if let Some(n) = with(|m| m.get(user_id).cloned()) {
        n.notify_waiters();
    }
}

/// O destinatario esta ligado com a app aberta: tem uma espera ativa ou acabou
/// uma ha menos de `ONLINE_GRACE` (a meio de receber e confirmar).
pub fn is_online(user_id: &str) -> bool {
    if is_waiting(user_id) {
        return true;
    }
    let now = Instant::now();
    with_recent(|m| m.get(user_id).is_some_and(|at| now.saturating_duration_since(*at) <= ONLINE_GRACE))
}

/// Chamado uma vez por minuto: limpa as esperas antigas.
pub fn prune_recent() {
    let now = Instant::now();
    with_recent(|m| prune(m, now));
}

/// Utilizadores com alguem a espera neste momento (para estatisticas).
pub fn waiting_users() -> usize {
    with(|m| m.len())
}

/// Ha alguem deste utilizador a espera (para testes).
pub fn is_waiting(user_id: &str) -> bool {
    with(|m| m.contains_key(user_id))
}
