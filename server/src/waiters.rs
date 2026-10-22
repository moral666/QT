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
use tokio::sync::Notify;

/// Espera maxima de um `WaitMessages`, em segundos. Abaixo dos tempos de
/// inatividade tipicos de proxies e operadoras moveis (30 s ou mais).
pub const MAX_WAIT_SECS: u64 = 25;

static WAITERS: Mutex<Option<HashMap<String, Arc<Notify>>>> = Mutex::new(None);

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

/// Utilizadores com alguem a espera neste momento (para estatisticas).
pub fn waiting_users() -> usize {
    with(|m| m.len())
}

/// Ha alguem deste utilizador a espera (testes e leituras telefone a telefone, `latency.rs`).
pub fn is_waiting(user_id: &str) -> bool {
    with(|m| m.contains_key(user_id))
}
