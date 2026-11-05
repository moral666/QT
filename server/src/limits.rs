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

use std::collections::{BTreeMap, HashMap};
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

// ---------------------------------------------------------------- ligacoes sem conta
//
// Quem se autentica mas ainda nao tem conta neste relay (a registar-se, ou a buscar a copia de
// recuperacao) ocupa uma vaga de um grupo pequeno: `max_unregistered` no total, e sao as
// primeiras a sair quando o relay enche.

static UNREGISTERED: Mutex<Option<BTreeMap<u64, Arc<Notify>>>> = Mutex::new(None);

fn with_unregistered<R>(f: impl FnOnce(&mut BTreeMap<u64, Arc<Notify>>) -> R) -> R {
    let mut guard = UNREGISTERED.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(BTreeMap::new))
}

/// Vaga de uma ligacao autenticada sem conta; liberta-se ao ser largada.
pub struct UnregisteredSlot(u64);

impl UnregisteredSlot {
    /// `None` se ja ha `max` ligacoes assim. `max = 0` desliga o limite.
    pub fn try_acquire(max: usize, close: Arc<Notify>) -> Option<Self> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        with_unregistered(|m| {
            if max > 0 && m.len() >= max {
                return None;
            }
            m.insert(id, close);
            Some(UnregisteredSlot(id))
        })
    }

    /// O relay esta cheio: fecha a ligacao sem conta mais antiga. `false`: nao ha nenhuma.
    pub fn close_oldest() -> bool {
        with_unregistered(|m| match m.pop_first() {
            Some((_, close)) => {
                close.notify_one();
                true
            }
            None => false,
        })
    }

    /// Ligacoes sem conta neste momento.
    pub fn now() -> usize {
        with_unregistered(|m| m.len())
    }
}

impl Drop for UnregisteredSlot {
    fn drop(&mut self) {
        with_unregistered(|m| {
            m.remove(&self.0);
        });
    }
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
    /// Quando a conta foi criada (ms desde 1970); 0 = conta antiga (sem marca no Redis).
    created_ms: AtomicU64,
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
        (UserSlot { user_id: user_id.to_string(), id, close, registered, created_ms: AtomicU64::new(0) }, closed)
    }

    /// Hora de criacao da conta (ver `Store::auth_state`). 0 = conta antiga.
    pub fn set_created_ms(&self, ms: u64) {
        self.created_ms.store(ms, Ordering::Relaxed);
    }

    /// A conta foi criada ha menos de `secs` segundos? Contas sem marca contam como antigas.
    pub fn younger_than(&self, secs: u64) -> bool {
        let at = self.created_ms.load(Ordering::Relaxed);
        at != 0 && crate::store::now_ms().saturating_sub(at) < secs.saturating_mul(1000)
    }

    /// Para fechar esta ligacao de fora (ver `UnregisteredSlot`).
    pub fn close_handle(&self) -> Arc<Notify> {
        self.close.clone()
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
    /// `RegisterPreKeyBundle` por conta.
    Register,
    /// Provas de trabalho verificadas, em todo o relay.
    Pow,
    /// Contas novas, em todo o relay.
    NewAccount,
    /// Bytes em fila por conta.
    SendBytes,
    /// Bytes de copias de recuperacao por conta.
    BackupBytes,
    /// Bytes enviados a uma conta.
    Egress,
    /// Paginas de mensagens entregues a uma conta.
    Fetch,
    /// Anexos novos de uma conta sem premium.
    BlobCreate,
    /// Perguntas a outro relay por anexos que ninguem conhece.
    FedLookup,
    /// One-time prekeys entregues de uma conta.
    Otpk,
    /// Notificacoes por dia a uma conta.
    Push,
}

/// `burst = 0` desliga o limite.
#[derive(Clone, Copy, Debug)]
pub struct Rate {
    pub burst: u32,
    pub per_minute: u32,
}

/// Limite por dia (bytes ou contagens). `per_day = 0` desliga-o.
#[derive(Clone, Copy, Debug)]
pub struct DailyRate {
    pub per_day: u64,
}

struct Bucket {
    tokens: f64,
    at: std::time::Instant,
    burst: f64,
    per_sec: f64,
}

impl Bucket {
    fn refilled(&self, now: std::time::Instant) -> f64 {
        (self.tokens + now.duration_since(self.at).as_secs_f64() * self.per_sec).min(self.burst)
    }
}

#[derive(Default)]
struct Buckets {
    map: HashMap<(Limited, String), Bucket>,
    pruned: Option<std::time::Instant>,
}

static BUCKETS: Mutex<Option<Buckets>> = Mutex::new(None);
/// Acima disto, as contas com o balde cheio saem do mapa, no maximo uma vez por minuto.
const MAX_TRACKED: usize = 50_000;
/// Acima disto saem tambem os baldes parados ha mais de uma hora.
const MAX_TRACKED_HARD: usize = 300_000;

/// Ultimo estado do balde depois de reposto. `charge`: gasta mesmo sem haver fichas (o custo so
/// se sabe depois, ex.: bytes ja enviados).
fn take(kind: Limited, key: &str, burst: f64, per_sec: f64, cost: f64, charge: bool) -> bool {
    let now = std::time::Instant::now();
    let mut guard = BUCKETS.lock().unwrap_or_else(|e| e.into_inner());
    let b = guard.get_or_insert_with(Buckets::default);
    if b.map.len() > MAX_TRACKED && b.pruned.is_none_or(|t| now.duration_since(t).as_secs() >= 60) {
        b.pruned = Some(now);
        b.map.retain(|_, e| e.refilled(now) < e.burst);
        if b.map.len() > MAX_TRACKED_HARD {
            b.map.retain(|_, e| now.duration_since(e.at).as_secs() < 3600);
        }
    }
    let e = b.map.entry((kind, key.to_string())).or_insert(Bucket { tokens: burst, at: now, burst, per_sec });
    (e.burst, e.per_sec) = (burst, per_sec);
    let tokens = e.refilled(now);
    e.at = now;
    if charge {
        e.tokens = (tokens - cost).max(-burst);
        return e.tokens > 0.0;
    }
    if tokens < cost {
        e.tokens = tokens;
        return false;
    }
    e.tokens = tokens - cost;
    true
}

/// Gasta uma ficha da conta. `false`: passou o limite (o pedido e recusado).
pub fn allow(kind: Limited, user_id: &str, rate: Rate) -> bool {
    if rate.burst == 0 {
        return true;
    }
    take(kind, user_id, rate.burst as f64, rate.per_minute as f64 / 60.0, 1.0, false)
}

/// Como `allow`, com o limite dado por hora (`per_hour` seguidos e ao longo da hora). 0 desliga-o.
pub fn allow_per_hour(kind: Limited, key: &str, per_hour: u32) -> bool {
    if per_hour == 0 {
        return true;
    }
    take(kind, key, per_hour as f64, per_hour as f64 / 3600.0, 1.0, false)
}

/// Gasta `cost` do limite diario. `false`: nao chega (nada e gasto).
pub fn allow_daily(kind: Limited, key: &str, rate: DailyRate, cost: u64) -> bool {
    if rate.per_day == 0 {
        return true;
    }
    take(kind, key, rate.per_day as f64, rate.per_day as f64 / 86_400.0, cost as f64, false)
}

/// Gasta `cost` do limite diario mesmo que nao chegue. `false`: o limite acabou.
pub fn charge_daily(kind: Limited, key: &str, rate: DailyRate, cost: u64) -> bool {
    if rate.per_day == 0 {
        return true;
    }
    take(kind, key, rate.per_day as f64, rate.per_day as f64 / 86_400.0, cost as f64, true)
}

// ------------------------------------------------------------------- memoria do Redis

/// Memoria do Redis usada, em percentagem de `maxmemory` (0 = desconhecida). Atualizada pelo
/// `relay_server` de 10 em 10 s.
static REDIS_PCT: AtomicU64 = AtomicU64::new(0);
/// Acima disto so as contas antigas escrevem: as contas novas esperam.
pub const REDIS_TIGHT_PCT: u64 = 80;

pub fn set_redis_pct(pct: u64) {
    REDIS_PCT.store(pct, Ordering::Relaxed);
}

pub fn redis_pct() -> u64 {
    REDIS_PCT.load(Ordering::Relaxed)
}

/// O Redis esta quase cheio?
pub fn redis_tight() -> bool {
    redis_pct() >= REDIS_TIGHT_PCT
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

    #[test]
    fn limite_diario_em_bytes() {
        let user = "a1".repeat(32);
        let rate = DailyRate { per_day: 1000 };
        assert!(allow_daily(Limited::SendBytes, &user, rate, 900));
        assert!(!allow_daily(Limited::SendBytes, &user, rate, 200), "nao chega");
        assert!(allow_daily(Limited::SendBytes, &user, rate, 90), "o pedido recusado nao gastou nada");
        assert!(allow_daily(Limited::Egress, &user, DailyRate { per_day: 0 }, u64::MAX), "0 = sem limite");
    }

    #[test]
    fn gastar_depois_de_enviar_passa_abaixo_de_zero() {
        let user = "a2".repeat(32);
        let rate = DailyRate { per_day: 100 };
        assert!(charge_daily(Limited::Egress, &user, rate, 50));
        assert!(!charge_daily(Limited::Egress, &user, rate, 150), "acabou");
        assert!(!charge_daily(Limited::Egress, &user, rate, 0), "continua sem fichas");
    }

    #[test]
    fn sem_conta_grupo_proprio_e_a_mais_antiga_sai_primeiro() {
        use std::sync::Arc;
        let base = UnregisteredSlot::now();
        let (a, b) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        let s1 = UnregisteredSlot::try_acquire(base + 2, a.clone()).unwrap();
        let s2 = UnregisteredSlot::try_acquire(base + 2, b).unwrap();
        assert!(UnregisteredSlot::try_acquire(base + 2, Arc::new(Notify::new())).is_none());
        drop(s2);
        assert!(UnregisteredSlot::try_acquire(0, Arc::new(Notify::new())).is_some(), "0 = sem limite");
        drop(s1);
    }
}
