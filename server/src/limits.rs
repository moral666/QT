//! Limites de ligacoes contra abusos (falha 3, parte B, aprovada a 19/10/2026).
//!
//! Partilhados por todas as ligacoes do processo:
//! - **Ligacoes por autenticar** (handshake Noise + autenticacao): no maximo
//!   `max_unauthenticated` ao mesmo tempo. Uma enxurrada de ligacoes que nunca se
//!   autenticam deixa de conseguir tirar a vaga a quem ja esta autenticado.
//! - **Ligacoes autenticadas por conta:** no maximo `max_connections_per_user`. Ao
//!   passar o maximo, fecha-se a ligacao MAIS ANTIGA dessa conta e a nova fica. Um
//!   telemovel que muda de rede (Wi-Fi <-> dados moveis) deixa ligacoes mortas para
//!   tras, que so caem pelo tempo de inatividade; assim nunca fica de fora por
//!   causa delas, e uma conta nunca acumula mais do que o maximo.
//!
//! Tudo so em memoria. O mapa por conta usa o `user_id` que o relay ja conhece de
//! cada ligacao autenticada (como `waiters.rs`): nada de novo e guardado nem registado.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

static UNAUTHENTICATED: AtomicUsize = AtomicUsize::new(0);

/// Vaga de uma ligacao que ainda nao se autenticou; liberta-se ao ser largada.
pub struct UnauthenticatedSlot(());

impl UnauthenticatedSlot {
    /// `None` se ja ha `max` ligacoes por autenticar. `max = 0` desliga o limite.
    pub fn try_acquire(max: usize) -> Option<Self> {
        let mut current = UNAUTHENTICATED.load(Ordering::Relaxed);
        loop {
            if max > 0 && current >= max {
                return None;
            }
            match UNAUTHENTICATED.compare_exchange_weak(current, current + 1, Ordering::AcqRel, Ordering::Relaxed) {
                Ok(_) => return Some(UnauthenticatedSlot(())),
                Err(v) => current = v,
            }
        }
    }
}

impl Drop for UnauthenticatedSlot {
    fn drop(&mut self) {
        UNAUTHENTICATED.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Ligacoes por autenticar neste momento.
pub fn unauthenticated_now() -> usize {
    UNAUTHENTICATED.load(Ordering::Relaxed)
}

struct Entry {
    id: u64,
    close: Arc<Notify>,
    registered: Arc<AtomicBool>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static BY_USER: Mutex<Option<HashMap<String, Vec<Entry>>>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut HashMap<String, Vec<Entry>>) -> R) -> R {
    let mut guard = BY_USER.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// Registo de uma ligacao autenticada. `close` dispara quando uma ligacao mais
/// nova da mesma conta a obriga a fechar. Ao ser largado, sai do registo.
pub struct UserSlot {
    user_id: String,
    id: u64,
    close: Arc<Notify>,
    /// A conta tem bundle neste relay (visto na autenticacao ou num registo desta
    /// ligacao): o `SendMessage` deixa de o verificar a cada mensagem. Apagar a conta ou a
    /// federacao larga-la limpa a marca em todas as ligacoes da conta (`forget_registered`);
    /// banir fecha-as (`close_all`). `false` quer dizer "verificar", nao "sem conta".
    registered: Arc<AtomicBool>,
}

impl UserSlot {
    /// Regista a ligacao e fecha as mais antigas da mesma conta acima de `max`.
    /// Devolve o registo e quantas foram fechadas. `max = 0` desliga o limite.
    pub fn register(user_id: &str, max: usize) -> (Self, usize) {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let close = Arc::new(Notify::new());
        let registered = Arc::new(AtomicBool::new(false));
        let closed = with(|m| {
            let list = m.entry(user_id.to_string()).or_default();
            let mut closed = 0;
            while max > 0 && list.len() >= max {
                // notify_one guarda o aviso mesmo que a ligacao ainda nao esteja a
                // ouvir: fecha logo que la chegue.
                list.remove(0).close.notify_one();
                closed += 1;
            }
            list.push(Entry { id, close: close.clone(), registered: registered.clone() });
            closed
        });
        (UserSlot { user_id: user_id.to_string(), id, close, registered }, closed)
    }

    /// A conta desta ligacao tem bundle (ver o campo `registered`).
    pub fn registered(&self) -> bool {
        self.registered.load(Ordering::Acquire)
    }

    pub fn set_registered(&self, yes: bool) {
        self.registered.store(yes, Ordering::Release);
    }

    /// A conta deixou de ter bundle aqui (apagada, ou levada para outro relay): as
    /// ligacoes dela voltam a verificar no proximo envio.
    pub fn forget_registered(user_id: &str) {
        with(|m| {
            if let Some(list) = m.get(user_id) {
                for e in list {
                    e.registered.store(false, Ordering::Release);
                }
            }
        })
    }

    /// Fecha todas as ligacoes abertas desta conta (conta banida pelo operador).
    /// Devolve quantas eram.
    pub fn close_all(user_id: &str) -> usize {
        with(|m| match m.get(user_id) {
            Some(list) => {
                for e in list {
                    e.close.notify_one();
                }
                list.len()
            }
            None => 0,
        })
    }

    /// Contas diferentes com pelo menos uma ligacao autenticada agora, sem contar
    /// `except` (o operador que pergunta). So o numero: o mapa nao sai daqui.
    pub fn users_online(except: Option<&str>) -> usize {
        with(|m| m.len() - except.map_or(0, |e| m.contains_key(e) as usize))
    }

    /// Completa quando esta ligacao tem de fechar (ha uma mais nova da mesma conta).
    pub async fn must_close(&self) {
        self.close.notified().await
    }
}

impl Drop for UserSlot {
    fn drop(&mut self) {
        with(|m| {
            if let Some(list) = m.get_mut(&self.user_id) {
                list.retain(|e| e.id != self.id);
                if list.is_empty() {
                    m.remove(&self.user_id);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn por_autenticar_respeita_o_maximo_e_liberta_ao_largar() {
        let base = unauthenticated_now();
        let a = UnauthenticatedSlot::try_acquire(base + 2).unwrap();
        let b = UnauthenticatedSlot::try_acquire(base + 2).unwrap();
        assert!(UnauthenticatedSlot::try_acquire(base + 2).is_none());
        drop(a);
        let c = UnauthenticatedSlot::try_acquire(base + 2).unwrap();
        drop((b, c));
        assert!(UnauthenticatedSlot::try_acquire(0).is_some(), "0 = sem limite");
    }

    #[tokio::test]
    async fn por_conta_fecha_a_mais_antiga() {
        let user = "ab".repeat(32);
        let (s1, c1) = UserSlot::register(&user, 2);
        let (s2, c2) = UserSlot::register(&user, 2);
        assert_eq!((c1, c2), (0, 0));
        let (s3, c3) = UserSlot::register(&user, 2);
        assert_eq!(c3, 1, "a terceira fecha a primeira");
        // A primeira recebe o aviso (mesmo que so agora comece a ouvir).
        tokio::time::timeout(std::time::Duration::from_millis(200), s1.must_close()).await.unwrap();
        assert!(tokio::time::timeout(std::time::Duration::from_millis(50), s2.must_close()).await.is_err());
        assert!(UserSlot::users_online(None) >= 1);
        drop((s1, s2, s3));
        assert!(with(|m| !m.contains_key(&user)), "a conta sai do mapa quando fecha tudo");
    }
}

// ---------------------------------------------------------------- pedidos por conta
//
// Limite de pedidos por conta e por minuto (balde de fichas): cada conta pode fazer
// ate `burst` pedidos seguidos e ganha `per_minute` fichas por minuto. So em memoria,
// so contagens: o relay nao guarda a quem cada um escreve.

/// Tipo de pedido limitado.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Limited {
    /// `SendMessage` (cada envelope conta: uma mensagem de grupo sao ate 9).
    Send,
    /// `FetchPreKeyBundle` (cada um gasta uma one-time prekey do contacto).
    FetchBundle,
    /// `BackupPut` e `BackupGet` (copia de recuperacao, partes ate 40 KB).
    Backup,
    /// `IssueFichas` (cada pedido sao ate 400 operacoes no grupo e uma prova).
    Fichas,
}

/// `burst = 0` desliga o limite.
#[derive(Clone, Copy, Debug)]
pub struct Rate {
    pub burst: u32,
    pub per_minute: u32,
}

type Buckets = HashMap<(Limited, String), (f64, std::time::Instant)>;
static BUCKETS: Mutex<Option<Buckets>> = Mutex::new(None);
/// Acima disto, as contas com o balde cheio (paradas) saem do mapa.
const MAX_TRACKED: usize = 50_000;

/// Gasta uma ficha da conta. `false`: passou o limite (o pedido e recusado).
pub fn allow(kind: Limited, user_id: &str, rate: Rate) -> bool {
    if rate.burst == 0 {
        return true;
    }
    let now = std::time::Instant::now();
    let (burst, per_sec) = (rate.burst as f64, rate.per_minute as f64 / 60.0);
    let mut guard = BUCKETS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if map.len() > MAX_TRACKED {
        map.retain(|_, (tokens, at)| *tokens + now.duration_since(*at).as_secs_f64() * per_sec < burst);
    }
    let entry = map.entry((kind, user_id.to_string())).or_insert((burst, now));
    let tokens = (entry.0 + now.duration_since(entry.1).as_secs_f64() * per_sec).min(burst);
    if tokens < 1.0 {
        *entry = (tokens, now);
        return false;
    }
    *entry = (tokens - 1.0, now);
    true
}

#[cfg(test)]
mod rate_tests {
    use super::*;

    #[test]
    fn balde_deixa_rajada_e_depois_recusa() {
        let user = "cd".repeat(32);
        let rate = Rate { burst: 3, per_minute: 1 };
        assert!((0..3).all(|_| allow(Limited::Send, &user, rate)));
        assert!(!allow(Limited::Send, &user, rate));
        // Outro tipo de pedido e outra conta tem baldes proprios.
        assert!(allow(Limited::FetchBundle, &user, rate));
        assert!(allow(Limited::Send, &"ef".repeat(32), rate));
        assert!(allow(Limited::Send, &user, Rate { burst: 0, per_minute: 0 }), "0 = sem limite");
    }
}
