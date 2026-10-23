//! Fichas do video pelo relay (patch 0006), do lado do telemovel que ENVIA video.
//!
//! O relay das chamadas so deixa passar o video de um lado com credito: uma ficha da um
//! bloco (`qt_core::fichas::BLOCK_SECS`, 1 minuto). Este modulo decide quando gastar:
//!
//!   * so quando o video SAI PELO RELAY (caminho direto nao gasta nada). Com a ligacao direta
//!     ainda a procurar caminho, espera ate `P2P_GRACE` antes da primeira ficha;
//!   * primeiro pergunta ao relay, na sala, a chave do mes (`key_request`). So se gastam fichas
//!     se essa chave for a PUBLICADA no site (`https://sealer.pt/estado.json`, fora do relay;
//!     `FichasConfig::published`) e se as fichas foram emitidas com ela. Um relay comprometido
//!     que desse a cada conta uma chave diferente (particao de chaves) nao passa daqui. Sem a
//!     chave do site, ou com outra: nada se gasta e o video pelo relay fica em pausa;
//!   * a ficha seguinte vai com `RENEW_BEFORE` de folga; sem resposta, repete a mesma (o
//!     relay reconhece-a e nao conta duas vezes);
//!   * sem fichas (ou so de outro mes): o video fica em pausa pelo
//!     relay, a voz continua, e a ligacao direta continua com video;
//!   * relay sem fichas (`KeyStatus::Free`) ou antigo (nao responde): video como antes.
//!
//! A carteira (`Wallet`) e da app (base cifrada); aqui so se pede a proxima e se avisa
//! quando uma ja nao serve.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qt_core::call::{RelayVideo, Role, RoomKeys};
use qt_core::fichas::{
    self, Ficha, KeyStatus, RedeemStatus, KEY_RESP_KIND, KEY_RESP_LEN, PUBLIC_KEY_LEN, REDEEM_ACK_KIND, REDEEM_ACK_LEN,
    REDEEM_KIND, REDEEM_PROLOGUE,
};

/// Fichas da app para esta chamada.
pub trait Wallet: Send + Sync {
    /// A proxima ficha ainda por gastar deste mes e desta chave (sem a tirar da carteira).
    fn next(&self, epoch: u32, key: &[u8; PUBLIC_KEY_LEN]) -> Option<Ficha>;
    /// Esta ficha ja nao serve (o relay aceitou-a, ou diz que ja estava gasta ou que e invalida).
    fn used(&self, nonce: &[u8; 32]);
}

/// Carteira em memoria (a app carrega as fichas da base cifrada e, no fim, apaga as usadas).
#[derive(Default)]
pub struct MemWallet {
    fichas: Mutex<Vec<Ficha>>,
    used: Mutex<Vec<[u8; 32]>>,
}

impl MemWallet {
    pub fn new(fichas: Vec<Ficha>) -> Arc<Self> {
        Arc::new(MemWallet { fichas: Mutex::new(fichas), used: Mutex::new(Vec::new()) })
    }

    /// As usadas desde a ultima vez (para a app as apagar da base).
    pub fn take_used(&self) -> Vec<[u8; 32]> {
        self.used.lock().map(|mut u| std::mem::take(&mut *u)).unwrap_or_default()
    }

    /// Fichas ainda por usar.
    pub fn left(&self) -> usize {
        self.fichas.lock().map(|f| f.len()).unwrap_or(0)
    }
}

impl Wallet for MemWallet {
    fn next(&self, epoch: u32, key: &[u8; PUBLIC_KEY_LEN]) -> Option<Ficha> {
        self.fichas.lock().ok()?.iter().find(|f| f.epoch == epoch && &f.issuer_key == key).cloned()
    }

    fn used(&self, nonce: &[u8; 32]) {
        if let Ok(mut f) = self.fichas.lock() {
            f.retain(|x| &x.nonce != nonce);
        }
        if let Ok(mut u) = self.used.lock() {
            if !u.contains(nonce) {
                u.push(*nonce);
            }
        }
    }
}

/// O que o motor precisa para gastar fichas.
#[derive(Clone)]
pub struct FichasConfig {
    pub wallet: Arc<dyn Wallet>,
    /// Chave estatica do relay (o pin): o resgate vai cifrado para ela.
    pub relay_public_key: Vec<u8>,
    /// Chaves das fichas publicadas no site (`(AAAAMM, chave)`), que a app foi buscar fora do
    /// relay. Vazio (site indisponivel desde sempre): nenhuma ficha se gasta.
    pub published: Vec<(u32, [u8; PUBLIC_KEY_LEN])>,
}

impl std::fmt::Debug for FichasConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FichasConfig(..)")
    }
}

/// Pede a ficha seguinte quando o credito baixa disto.
pub const RENEW_BEFORE: Duration = Duration::from_secs(15);
/// Com a ligacao direta a procurar caminho, espera isto antes de gastar a primeira ficha.
pub const P2P_GRACE: Duration = Duration::from_secs(4);
const KEY_RETRY: Duration = Duration::from_millis(500);
/// Sem resposta a tantos pedidos da chave: relay antigo (sem fichas), video como antes.
const KEY_TRIES: u32 = 4;
const REDEEM_RETRY: Duration = Duration::from_millis(700);
const REDEEM_TRIES: u32 = 6;
/// O relay nao conseguiu verificar (ou nao respondeu): tenta outra vez daqui a isto.
const BACKOFF: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Relay {
    /// Ainda nao se perguntou (so se pergunta quando for preciso).
    Unknown,
    Asking { tries: u32, next: Instant },
    /// O relay nao pede fichas (ou e antigo): o video passa como antes.
    Free,
    Required { epoch: u32, key: [u8; PUBLIC_KEY_LEN] },
    /// Pede fichas mas agora nao as verifica: perguntar outra vez mais tarde.
    Unavailable { until: Instant },
    /// A chave da sala nao e a publicada no site (ou a app nao a tem): nada se gasta.
    Mismatch,
}

struct Pending {
    ficha: Ficha,
    packet: Vec<u8>,
    ack_id: [u8; 4],
    sent: Instant,
    tries: u32,
}

/// Estado das fichas de uma chamada (thread da rede).
pub(crate) struct Fichas {
    cfg: Option<FichasConfig>,
    room: RoomKeys,
    relay: Relay,
    credit_until: Option<Instant>,
    pending: Option<Pending>,
    /// Desde quando o video quer sair pelo relay sem credito (para a espera da ligacao direta).
    need_since: Option<Instant>,
    /// Nao ha fichas que sirvam (acabaram, ou sao de outro mes ou de outra chave).
    empty: bool,
    backoff_until: Option<Instant>,
    /// Fichas aceites nesta chamada.
    pub(crate) spent: u64,
}

/// Efeito de uma resposta do relay no controlo do video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Effect {
    /// O relay deu credito: modo do video agora (normal, 240p ou sem video pela sala).
    Mode(RelayVideo),
}

impl Fichas {
    pub(crate) fn new(cfg: Option<FichasConfig>, room: RoomKeys) -> Self {
        Fichas { cfg, room, relay: Relay::Unknown, credit_until: None, pending: None, need_since: None, empty: false, backoff_until: None, spent: 0 }
    }

    /// O video pode sair pelo relay agora? (Relay sem fichas, ou credito por gastar.)
    pub(crate) fn relay_ok(&self, now: Instant) -> bool {
        matches!(self.relay, Relay::Free) || self.credit_until.is_some_and(|u| now < u)
    }

    /// Sem credito e sem maneira de o ter: o video pelo relay fica em pausa (teto da conta).
    pub(crate) fn exhausted(&self, now: Instant) -> bool {
        if self.relay_ok(now) || self.pending.is_some() {
            return false;
        }
        match self.relay {
            Relay::Required { .. } => self.empty || self.cfg.is_none(),
            Relay::Unavailable { .. } | Relay::Mismatch => true,
            _ => false,
        }
    }

    /// Segundos de credito que restam (para as contas).
    pub(crate) fn credit_secs(&self, now: Instant) -> u32 {
        self.credit_until.map_or(0, |u| u.saturating_duration_since(now).as_secs() as u32)
    }

    /// A cada 100 ms. `want`: o video quer sair SO pelo relay. `p2p_searching`: a ligacao direta
    /// esta ligada mas ainda sem caminho. `role`: `None` antes de atender. Devolve um pacote
    /// para o relay, se houver.
    pub(crate) fn tick(&mut self, now: Instant, joined: bool, want: bool, p2p_searching: bool, role: Option<Role>) -> Option<Vec<u8>> {
        if !joined {
            return None;
        }
        // A chave do mes pergunta-se logo ao entrar na sala (ainda a tocar), antes de ser precisa:
        // assim o video nunca espera por ela, e se o caminho direto cair passa logo ao relay.
        match self.relay {
            Relay::Unknown => return self.on_joined(now),
            Relay::Asking { tries, next } if now >= next => {
                if tries >= KEY_TRIES {
                    // Relay antigo: nao conhece fichas (nem as pede).
                    self.relay = Relay::Free;
                } else {
                    self.relay = Relay::Asking { tries: tries + 1, next: now + KEY_RETRY };
                    return Some(fichas::key_request(&self.room).to_vec());
                }
            }
            Relay::Unavailable { until } if want && now >= until => {
                self.relay = Relay::Asking { tries: 1, next: now + KEY_RETRY };
                return Some(fichas::key_request(&self.room).to_vec());
            }
            _ => {}
        }
        if !want {
            self.need_since = None;
            // A resposta a uma ficha ja enviada continua a contar.
            return self.retry_pending(now);
        }
        if self.relay_ok(now) && self.credit_until.is_none_or(|u| u.saturating_duration_since(now) > RENEW_BEFORE) {
            return self.retry_pending(now);
        }
        let since = *self.need_since.get_or_insert(now);
        let Relay::Required { epoch, key } = self.relay else { return None };
        if self.pending.is_some() {
            return self.retry_pending(now);
        }
        if self.backoff_until.is_some_and(|b| now < b) {
            return None;
        }
        // Ligacao direta a procurar: talvez nem seja preciso gastar.
        if p2p_searching && now.duration_since(since) < P2P_GRACE && self.credit_until.is_none() {
            return None;
        }
        let (Some(cfg), Some(role)) = (self.cfg.as_ref(), role) else {
            self.empty = true;
            return None;
        };
        let Some(ficha) = cfg.wallet.next(epoch, &key) else {
            self.empty = true;
            return None;
        };
        let payload = fichas::redeem_payload(role, &self.room, &ficha);
        let sealed = qt_transport::noise_n_seal(&cfg.relay_public_key, REDEEM_PROLOGUE, &payload).ok()?;
        let mut packet = Vec::with_capacity(1 + sealed.len());
        packet.push(REDEEM_KIND);
        packet.extend_from_slice(&sealed);
        let ack_id = fichas::ack_id(&ficha.nonce);
        self.pending = Some(Pending { ficha, packet: packet.clone(), ack_id, sent: now, tries: 1 });
        Some(packet)
    }

    /// Acabou de entrar na sala: pergunta ja a chave do mes (uma vez).
    pub(crate) fn on_joined(&mut self, now: Instant) -> Option<Vec<u8>> {
        if self.relay != Relay::Unknown {
            return None;
        }
        self.relay = Relay::Asking { tries: 1, next: now + KEY_RETRY };
        Some(fichas::key_request(&self.room).to_vec())
    }

    fn retry_pending(&mut self, now: Instant) -> Option<Vec<u8>> {
        let p = self.pending.as_mut()?;
        if now.duration_since(p.sent) < REDEEM_RETRY {
            return None;
        }
        if p.tries >= REDEEM_TRIES {
            // Sem resposta: a ficha fica na carteira (o relay pode nem a ter recebido).
            self.pending = None;
            self.backoff_until = Some(now + BACKOFF);
            return None;
        }
        p.tries += 1;
        p.sent = now;
        Some(p.packet.clone())
    }

    /// E uma resposta do relay sobre fichas? (Barato, sem chaves.)
    pub(crate) fn is_fichas_packet(data: &[u8]) -> bool {
        (data.len() == REDEEM_ACK_LEN && data[0] == REDEEM_ACK_KIND) || (data.len() == KEY_RESP_LEN && data[0] == KEY_RESP_KIND)
    }

    /// Resposta do relay (chave do mes ou resposta a uma ficha).
    pub(crate) fn on_packet(&mut self, data: &[u8], now: Instant) -> Option<Effect> {
        if data[0] == KEY_RESP_KIND {
            let (epoch, status, key) = fichas::parse_key_response(&self.room, data)?;
            if !matches!(self.relay, Relay::Asking { .. }) {
                return None;
            }
            self.relay = match status {
                KeyStatus::Free => Relay::Free,
                // So a chave publicada no site para este mes (nunca a que o relay diz sozinho).
                KeyStatus::Required if self.cfg.as_ref().is_some_and(|c| c.published.contains(&(epoch, key))) => {
                    Relay::Required { epoch, key }
                }
                KeyStatus::Required => Relay::Mismatch,
                KeyStatus::Unavailable => Relay::Unavailable { until: now + BACKOFF },
            };
            return None;
        }
        let ack = fichas::parse_redeem_ack(&self.room, data)?;
        let p = self.pending.as_ref().filter(|p| p.ack_id == ack.id)?;
        let nonce = p.ficha.nonce;
        let wallet = self.cfg.as_ref().map(|c| c.wallet.clone());
        self.pending = None;
        match ack.status {
            RedeemStatus::Accepted | RedeemStatus::Repeated => {
                if let Some(w) = wallet {
                    w.used(&nonce);
                }
                self.spent += (ack.status == RedeemStatus::Accepted) as u64;
                self.credit_until = Some(now + Duration::from_secs(ack.credit_secs as u64));
                self.backoff_until = None;
                Some(Effect::Mode(ack.mode))
            }
            RedeemStatus::Spent | RedeemStatus::Invalid => {
                // Esta nao serve (gasta noutra altura, ou estragada): a proxima, ja no tique seguinte.
                if let Some(w) = wallet {
                    w.used(&nonce);
                }
                None
            }
            RedeemStatus::Unavailable => {
                self.backoff_until = Some(now + BACKOFF);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qt_core::fichas::{redeem_ack, IssueRequest, Issuer, RedeemAck};

    const SEED: [u8; 32] = [3u8; 32];

    fn room() -> RoomKeys {
        RoomKeys::derive(&[1; 32], &[2; 16])
    }

    fn wallet(epoch: u32, n: usize) -> (Arc<MemWallet>, [u8; 32]) {
        let iss = Issuer::derive(&SEED, epoch).unwrap();
        let (req, blinded) = IssueRequest::new(epoch, n).unwrap();
        let (ev, proof) = iss.issue(&blinded).unwrap();
        (MemWallet::new(req.finalize(&iss.public_key(), &ev, &proof).unwrap()), iss.public_key())
    }

    /// Carteira com a chave publicada no site para 202610 (a do emissor de teste).
    fn fichas_with(w: Arc<MemWallet>) -> (Fichas, Vec<u8>) {
        let published = [202609, 202610].iter().map(|e| (*e, Issuer::derive(&SEED, *e).unwrap().public_key())).collect();
        fichas_published(w, published)
    }

    fn fichas_published(w: Arc<MemWallet>, published: Vec<(u32, [u8; 32])>) -> (Fichas, Vec<u8>) {
        let relay = qt_transport::generate_static_keypair().unwrap();
        let f = Fichas::new(Some(FichasConfig { wallet: w, relay_public_key: relay.public.clone(), published }), room());
        (f, relay.private)
    }

    fn key_resp(epoch: u32, status: KeyStatus, key: &[u8; 32]) -> Vec<u8> {
        fichas::key_response(&room(), epoch, status, key).to_vec()
    }

    /// O relay le o resgate (como o relay das chamadas faz).
    fn opened(private: &[u8], p: &[u8]) -> fichas::RedeemRequest {
        assert_eq!(p[0], REDEEM_KIND);
        let plain = qt_transport::noise_n_open(private, REDEEM_PROLOGUE, &p[1..]).unwrap();
        fichas::parse_redeem_payload(&plain).unwrap()
    }

    fn ack(nonce: &[u8; 32], status: RedeemStatus, secs: u16) -> Vec<u8> {
        redeem_ack(&room(), &RedeemAck { role: Role::Caller, id: fichas::ack_id(nonce), status, mode: RelayVideo::Full, credit_secs: secs }).to_vec()
    }

    #[test]
    fn pede_a_chave_gasta_uma_ficha_e_renova_antes_do_fim() {
        let t = Instant::now();
        let (w, key) = wallet(202610, 3);
        let (mut f, private) = fichas_with(w.clone());
        // Fora da sala: nada.
        assert!(f.tick(t, false, true, false, Some(Role::Caller)).is_none());
        // Na sala (mesmo a tocar, antes de haver video): a chave do mes, logo, uma vez.
        let q = f.on_joined(t).unwrap();
        assert!(f.on_joined(t).is_none());
        assert!(f.tick(t, true, false, false, None).is_none(), "ja perguntou");
        assert_eq!(fichas::key_request_room(&q), Some(room().room_id));
        assert!(f.on_packet(&key_resp(202610, KeyStatus::Required, &key), t).is_none());
        assert!(!f.relay_ok(t) && !f.exhausted(t));
        // Sem video pelo relay nada se gasta.
        assert!(f.tick(t, true, false, false, Some(Role::Caller)).is_none());
        assert_eq!(w.left(), 3);
        // Depois a ficha (cifrada para o relay).
        let p = f.tick(t, true, true, false, Some(Role::Caller)).unwrap();
        let r = opened(&private, &p);
        assert_eq!((r.role, r.epoch), (Role::Caller, 202610));
        // Sem resposta: a mesma outra vez (nao gasta outra).
        assert!(f.tick(t + Duration::from_millis(100), true, true, false, Some(Role::Caller)).is_none());
        let p2 = f.tick(t + Duration::from_millis(800), true, true, false, Some(Role::Caller)).unwrap();
        assert_eq!(p2, p);
        // Aceite: 60 s de credito; a ficha sai da carteira.
        assert_eq!(f.on_packet(&ack(&r.nonce, RedeemStatus::Accepted, 60), t), Some(Effect::Mode(RelayVideo::Full)));
        assert!(f.relay_ok(t));
        assert_eq!((w.left(), w.take_used(), f.spent), (2, vec![r.nonce], 1));
        // Com credito folgado nada sai; a 15 s do fim pede a seguinte.
        assert!(f.tick(t + Duration::from_secs(30), true, true, false, Some(Role::Caller)).is_none());
        let p3 = f.tick(t + Duration::from_secs(46), true, true, false, Some(Role::Caller)).unwrap();
        assert_ne!(opened(&private, &p3).nonce, r.nonce);
    }

    #[test]
    fn gasta_ou_invalida_passa_a_seguinte_e_sem_fichas_pausa() {
        let t = Instant::now();
        let (w, key) = wallet(202610, 2);
        let (mut f, private) = fichas_with(w.clone());
        f.tick(t, true, true, false, Some(Role::Callee));
        f.on_packet(&key_resp(202610, KeyStatus::Required, &key), t);
        let a = opened(&private, &f.tick(t, true, true, false, Some(Role::Callee)).unwrap());
        // Resposta de outra ficha (ou adulterada): ignorada.
        assert!(f.on_packet(&ack(&[0u8; 32], RedeemStatus::Accepted, 60), t).is_none());
        assert!(!f.relay_ok(t));
        // Ja gasta (ex.: a app morreu a meio de uma chamada): sai da carteira, vai a seguinte.
        f.on_packet(&ack(&a.nonce, RedeemStatus::Spent, 0), t);
        let b = opened(&private, &f.tick(t, true, true, false, Some(Role::Callee)).unwrap());
        assert_ne!(a.nonce, b.nonce);
        f.on_packet(&ack(&b.nonce, RedeemStatus::Invalid, 0), t);
        // Acabaram: pausa pelo relay (a app nao mostra texto).
        assert!(f.tick(t, true, true, false, Some(Role::Callee)).is_none());
        assert!(f.exhausted(t) && !f.relay_ok(t));
        assert_eq!(w.left(), 0);
    }

    #[test]
    fn fichas_de_outro_mes_ou_de_outra_chave_nao_servem() {
        let t = Instant::now();
        let (w, key) = wallet(202609, 2);
        let (mut f, _) = fichas_with(w.clone());
        f.tick(t, true, true, false, Some(Role::Caller));
        f.on_packet(&key_resp(202610, KeyStatus::Required, &key), t);
        assert!(f.tick(t, true, true, false, Some(Role::Caller)).is_none(), "de setembro, em outubro");
        assert!(f.exhausted(t));
        // Do mes certo mas com uma chave diferente da que o relay diz a toda a gente (um emissor
        // que tentasse marcar esta conta com uma chave so dela): tambem nao.
        let (w2, _) = wallet(202610, 2);
        let (mut f2, _) = fichas_with(w2.clone());
        f2.tick(t, true, true, false, Some(Role::Caller));
        f2.on_packet(&key_resp(202610, KeyStatus::Required, &[9u8; 32]), t);
        assert!(f2.tick(t, true, true, false, Some(Role::Caller)).is_none());
        assert!(f2.exhausted(t));
        assert_eq!(w2.left(), 2, "nada foi gasto");
    }

    /// Particao de chaves: o relay diz a esta sala uma chave diferente da publicada no site (e a
    /// app ate tem fichas emitidas com essa chave, dadas pelo mesmo relay comprometido).
    #[test]
    fn chave_da_sala_diferente_da_publicada_nada_se_gasta() {
        let t = Instant::now();
        let iss = Issuer::derive(&[0x66; 32], 202610).unwrap();
        let (req, blinded) = IssueRequest::new(202610, 2).unwrap();
        let (ev, proof) = iss.issue(&blinded).unwrap();
        let w = MemWallet::new(req.finalize(&iss.public_key(), &ev, &proof).unwrap());
        let (mut f, _) = fichas_with(w.clone());
        f.on_joined(t);
        f.on_packet(&key_resp(202610, KeyStatus::Required, &iss.public_key()), t);
        assert!(f.tick(t, true, true, false, Some(Role::Caller)).is_none());
        assert!(f.exhausted(t) && !f.relay_ok(t), "video pelo relay em pausa");
        assert_eq!(w.left(), 2, "nada gasto");
    }

    /// Site indisponivel (a app nunca obteve a chave publicada): falha fechada.
    #[test]
    fn sem_chave_do_site_falha_fechada() {
        let t = Instant::now();
        let (w, key) = wallet(202610, 2);
        let (mut f, _) = fichas_published(w.clone(), Vec::new());
        f.on_joined(t);
        f.on_packet(&key_resp(202610, KeyStatus::Required, &key), t);
        assert!(f.tick(t, true, true, false, Some(Role::Caller)).is_none());
        assert!(f.exhausted(t));
        assert_eq!(w.left(), 2);
    }

    #[test]
    fn relay_sem_fichas_ou_antigo_video_como_antes() {
        let t = Instant::now();
        let (w, _) = wallet(202610, 1);
        let (mut f, _) = fichas_with(w.clone());
        f.tick(t, true, true, false, Some(Role::Caller));
        f.on_packet(&key_resp(202610, KeyStatus::Free, &[0u8; 32]), t);
        assert!(f.relay_ok(t) && f.tick(t, true, true, false, Some(Role::Caller)).is_none());
        // Relay antigo: nao responde a chave. Ao fim de 4 pedidos, video como antes.
        let (mut g, _) = fichas_with(w.clone());
        let mut asked = 0;
        for i in 0..30 {
            if g.tick(t + Duration::from_millis(100 * i), true, true, false, Some(Role::Caller)).is_some() {
                asked += 1;
            }
        }
        assert_eq!(asked, 4);
        assert!(g.relay_ok(t + Duration::from_secs(3)));
        assert_eq!(w.left(), 1, "nada gasto");
    }

    #[test]
    fn ligacao_direta_a_procurar_espera_antes_de_gastar() {
        let t = Instant::now();
        let (w, key) = wallet(202610, 1);
        let (mut f, _) = fichas_with(w.clone());
        f.tick(t, true, true, true, Some(Role::Caller));
        f.on_packet(&key_resp(202610, KeyStatus::Required, &key), t);
        assert!(f.tick(t + Duration::from_secs(1), true, true, true, Some(Role::Caller)).is_none());
        // Encontrou caminho direto: o video deixa de querer o relay, nada gasto.
        assert!(f.tick(t + Duration::from_secs(2), true, false, false, Some(Role::Caller)).is_none());
        assert_eq!(w.left(), 1);
        // Se nao encontrar ao fim da espera, gasta.
        assert!(f.tick(t + Duration::from_secs(3), true, true, true, Some(Role::Caller)).is_none());
        assert!(f.tick(t + Duration::from_secs(8), true, true, true, Some(Role::Caller)).is_some());
    }

    #[test]
    fn relay_sem_verificacao_nao_gasta_e_tenta_mais_tarde() {
        let t = Instant::now();
        let (w, key) = wallet(202610, 1);
        let (mut f, private) = fichas_with(w.clone());
        f.tick(t, true, true, false, Some(Role::Caller));
        f.on_packet(&key_resp(202610, KeyStatus::Unavailable, &[0u8; 32]), t);
        assert!(f.exhausted(t), "sem video pelo relay por agora");
        assert!(f.tick(t + Duration::from_secs(1), true, true, false, Some(Role::Caller)).is_none());
        // Mais tarde pergunta de novo.
        assert!(f.tick(t + Duration::from_secs(11), true, true, false, Some(Role::Caller)).is_some());
        f.on_packet(&key_resp(202610, KeyStatus::Required, &key), t + Duration::from_secs(11));
        let p = opened(&private, &f.tick(t + Duration::from_secs(11), true, true, false, Some(Role::Caller)).unwrap());
        // O registo falhou no relay: a ficha fica.
        f.on_packet(&ack(&p.nonce, RedeemStatus::Unavailable, 0), t + Duration::from_secs(11));
        assert_eq!(w.left(), 1);
        assert!(f.tick(t + Duration::from_secs(12), true, true, false, Some(Role::Caller)).is_none(), "espera");
        assert!(f.tick(t + Duration::from_secs(22), true, true, false, Some(Role::Caller)).is_some());
    }

    #[test]
    fn app_sem_carteira_fica_sem_video_pelo_relay() {
        let t = Instant::now();
        let mut f = Fichas::new(None, room());
        f.tick(t, true, true, false, Some(Role::Caller));
        f.on_packet(&key_resp(202610, KeyStatus::Required, &[1u8; 32]), t);
        assert!(f.tick(t, true, true, false, Some(Role::Caller)).is_none());
        assert!(f.exhausted(t));
    }
}
