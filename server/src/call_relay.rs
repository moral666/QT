//! Relay das CHAMADAS DE VOZ: reencaminha os pacotes de audio (ja cifrados ponta a
//! ponta pelos telemoveis) entre os dois lados de uma sala. Desenho aprovado pelo
//! Nuno a 19/10/2026 (A1 + B1), ver docs/protocol-spec.md, seccao 18.
//!
//!   * SALAS ANONIMAS. Um telemovel entra com um pedido cifrado para a chave estatica
//!     do relay (Noise_N, `qt_core::call`): o relay fica a saber o codigo da sala, o
//!     papel (quem liga / quem atende) e a chave das etiquetas. Nunca sabe a conta, e
//!     este processo nem tem acesso ao Redis.
//!   * UDP 443 e, se o UDP estiver bloqueado, uma ligacao WebSocket SEM CONTA
//!     (`wss://relay.sealer.pt/call`, pelo HAProxy) com os mesmos pacotes. Os dois
//!     lados podem estar em transportes diferentes.
//!   * MUDANCA DE IP (Wi-Fi para 4G): um pacote com etiqueta valida e contador maior
//!     do que o ultimo visto, vindo de outro endereco, muda o endereco desse lado (como
//!     no WireGuard). Um pacote antigo repetido de outro sitio nao desvia nada.
//!   * Nada decifra o audio. Sem registos com IPs: so contagens por minuto.
//!   * LIGACAO DIRETA (fase 5): quem ja esta numa sala pode perguntar "que endereco
//!     ves?" (`addr_request`, com a etiqueta da sala). O relay responde com o IP e a
//!     porta de onde veio o pedido, em menos bytes. Sem sala ou sem etiqueta valida, nada.
//!     O relay ja via esse IP: nao fica a saber mais nada.
//!
//!   * VIDEO (fase 5a): pacotes `0x40|papel` (297 a 1225 bytes), pelos mesmos lados da
//!     sala, so do endereco atual de cada lado (a mudanca de IP e pelo audio). Debito
//!     limitado por lado (balde de bytes, 150 kB/s por omissao, ~1,2 Mbit/s: 360p mais
//!     folga). De 5 em 5 s, quem envia video recebe um aviso (`relay_video_notice`): normal,
//!     so 240p, ou sem video.
//!   * TRAVAO DO MES: se o que o relay enviou no mes (UTC) passar de um valor (80 GB por
//!     omissao), o video desce a 240p para todos ate ao fim do mes (aviso + balde de
//!     50 kB/s). A conta do mes fica num ficheiro (`state_file`), gravado de 10 em 10 s.
//!   * TETO DA CONTA (horas de video por mes): ainda nao ha fichas anonimas (sessao propria).
//!     Fica a porta: uma sala com `video_allowed = false` so passa voz e quem envia video
//!     recebe "sem video pelo relay" (`CallLimits::relay_video = false` fecha-a para todas).
//!
//! LIMITES (para nao servir de tunel gratis): 2 lados por sala; a sala fecha se o
//! segundo nao entrar em 60 s ou ao fim de 30 s sem pacotes; salas por IP e no total;
//! pedidos de entrada por IP; pacotes por segundo por lado; tamanhos validos. O
//! relay nunca responde com mais bytes do que recebeu.
//!
//! A logica (`CallRelay`) nao faz rede (testes simples); `serve` liga-a aos sockets.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qt_core::call::{
    addr_ok, addr_request_room, join_ok, parse_header, parse_join_payload, parse_video_header, relay_tag_valid,
    relay_video_notice, RelayVideo, Role, ADDR_KIND, ADDR_LEN, JOIN_KIND, JOIN_PAYLOAD_LEN, JOIN_PROLOGUE, MAX_PAD,
    MEDIA_OVERHEAD, ROOM_ID_LEN, VIDEO_KIND, VIDEO_MAX_PACKET,
};
use zeroize::Zeroizing;

/// Tamanho exato de um pedido de entrada.
pub const JOIN_LEN: usize = 1 + qt_transport::NOISE_N_OVERHEAD + JOIN_PAYLOAD_LEN;
/// Maior pacote de audio aceite (com o maior `pad`).
pub const MAX_AUDIO_PACKET: usize = MEDIA_OVERHEAD + MAX_PAD;
/// Maior pacote aceite (o de video e o maior).
pub const MAX_PACKET: usize = if VIDEO_MAX_PACKET > MAX_AUDIO_PACKET { VIDEO_MAX_PACKET } else { MAX_AUDIO_PACKET };
/// Aviso sobre o video a quem o envia, no maximo de tanto em tanto tempo.
const VIDEO_NOTICE_EVERY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Endpoint {
    Udp(SocketAddr),
    /// Ligacao WebSocket (reserva quando o UDP esta bloqueado), pelo numero.
    Ws(u64),
}

#[derive(Debug, Clone)]
pub struct CallLimits {
    pub max_rooms: usize,
    /// Nas redes moveis muitos clientes saem pelo mesmo IP (CGNAT): nao pode ser baixo.
    pub rooms_per_ip: u32,
    /// Pedidos de entrada por IP a cada 10 s (cada um custa uma operacao X25519).
    pub joins_per_ip: u32,
    /// Pacotes por segundo de cada lado (o audio usa 50).
    pub packets_per_sec: u32,
    pub join_timeout: Duration,
    pub idle_timeout: Duration,
    /// Video por lado, em bytes/s (balde com 1 s de reserva). 150 000 = ~1,2 Mbit/s.
    pub video_bytes_per_sec: u32,
    /// O mesmo com o travao do mes (240p).
    pub video_cap240_bytes_per_sec: u32,
    /// Travao do mes: passado isto (bytes enviados pelo relay no mes, UTC), video so a 240p.
    pub monthly_egress_bytes: u64,
    /// `false`: nenhuma sala passa video (so voz). As fichas da conta vao decidir por sala.
    pub relay_video: bool,
    /// Onde guardar a conta do mes (sobrevive a reinicios). `None`: so em memoria.
    pub state_file: Option<std::path::PathBuf>,
}

impl Default for CallLimits {
    fn default() -> Self {
        CallLimits {
            max_rooms: 2_000,
            rooms_per_ip: 20,
            joins_per_ip: 60,
            packets_per_sec: 100,
            join_timeout: Duration::from_secs(60),
            idle_timeout: Duration::from_secs(30),
            video_bytes_per_sec: 150_000,
            video_cap240_bytes_per_sec: 50_000,
            monthly_egress_bytes: 80_000_000_000,
            relay_video: true,
            state_file: None,
        }
    }
}

/// Contagens anonimas (uma linha por minuto no registo).
#[derive(Default)]
pub struct CallStats {
    pub joins: AtomicU64,
    pub joins_rejected: AtomicU64,
    pub rooms_opened: AtomicU64,
    pub forwarded: AtomicU64,
    pub dropped: AtomicU64,
    pub roamed: AtomicU64,
    /// Respostas "que endereco ves?" (ligacao direta).
    pub addr_answered: AtomicU64,
    pub video_forwarded: AtomicU64,
    /// Video deitado fora: acima do debito do lado, ou sala sem video.
    pub video_dropped: AtomicU64,
}

struct Slot {
    ep: Endpoint,
    last_counter: Option<u64>,
    tokens: f64,
    refill_at: Instant,
    video_tokens: f64,
    video_refill_at: Instant,
    /// Ultimo aviso sobre o video a este lado (quando e qual).
    video_notice: Option<(Instant, RelayVideo)>,
}

impl Slot {
    fn new(ep: Endpoint, pps: u32, video_bps: u32, now: Instant) -> Self {
        Slot { ep, last_counter: None, tokens: pps as f64, refill_at: now, video_tokens: video_bps as f64, video_refill_at: now, video_notice: None }
    }
}

/// Bytes enviados pelo relay no mes (UTC), para o travao.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Egress {
    /// AAAAMM.
    pub month: u32,
    pub bytes: u64,
}

/// AAAAMM (UTC) de um instante em segundos desde 1970.
pub fn month_of_unix(secs: u64) -> u32 {
    // Data civil a partir dos dias (algoritmo de Howard Hinnant).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y * 100 + m) as u32
}

fn current_month() -> u32 {
    month_of_unix(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

struct Room {
    relay_key: Zeroizing<[u8; 32]>,
    slots: [Option<Slot>; 2],
    created: Instant,
    last_activity: Instant,
    creator_ip: IpAddr,
    /// Teto da conta (fichas anonimas, sessao propria): `false` = so voz nesta sala.
    video_allowed: bool,
}

pub struct CallRelay {
    static_private: Zeroizing<Vec<u8>>,
    limits: CallLimits,
    rooms: HashMap<[u8; ROOM_ID_LEN], Room>,
    rooms_by_ip: HashMap<IpAddr, u32>,
    join_rate: HashMap<IpAddr, (Instant, u32)>,
    egress: Egress,
    pub stats: Arc<CallStats>,
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn idx(role: Role) -> usize {
    role as usize
}

impl CallRelay {
    pub fn new(static_private: &[u8], limits: CallLimits) -> Self {
        CallRelay {
            static_private: Zeroizing::new(static_private.to_vec()),
            limits,
            rooms: HashMap::new(),
            rooms_by_ip: HashMap::new(),
            join_rate: HashMap::new(),
            egress: Egress { month: current_month(), bytes: 0 },
            stats: Arc::new(CallStats::default()),
        }
    }

    /// Bytes enviados no mes (para gravar e para a linha por minuto).
    pub fn egress(&self) -> Egress {
        self.egress
    }

    /// Repoe a conta do mes (lida do ficheiro ao arrancar). De outro mes: comeca do zero.
    pub fn set_egress(&mut self, e: Egress) {
        self.egress = if e.month == self.egress.month { e } else { Egress { month: self.egress.month, bytes: 0 } };
    }

    /// Muda de mes (chamar de vez em quando; `sweep` faz isso).
    pub fn roll_month(&mut self, month: u32) {
        if month != self.egress.month {
            self.egress = Egress { month, bytes: 0 };
        }
    }

    /// O que passa de video nesta sala agora.
    fn video_mode(&self, room_video_allowed: bool) -> RelayVideo {
        if !self.limits.relay_video || !room_video_allowed {
            RelayVideo::Denied
        } else if self.egress.bytes >= self.limits.monthly_egress_bytes {
            RelayVideo::Cap240
        } else {
            RelayVideo::Full
        }
    }

    pub fn rooms(&self) -> usize {
        self.rooms.len()
    }

    /// Um pacote chegou de `from` (IP real `ip`). Devolve o que enviar e para onde.
    pub fn handle(&mut self, data: &[u8], from: Endpoint, ip: IpAddr, now: Instant) -> Vec<(Endpoint, Vec<u8>)> {
        let out = match data.first() {
            Some(&JOIN_KIND) if data.len() == JOIN_LEN => self.on_join(data, from, ip, now),
            Some(&ADDR_KIND) if data.len() == ADDR_LEN => self.on_addr(data, from, ip, now),
            Some(k) if k & !1 == VIDEO_KIND && data.len() <= VIDEO_MAX_PACKET => self.on_video(data, from, now),
            Some(_) if data.len() <= MAX_AUDIO_PACKET => self.on_media(data, from, now),
            _ => Vec::new(),
        };
        if out.is_empty() {
            self.stats.dropped.fetch_add(1, Relaxed);
        }
        self.egress.bytes += out.iter().map(|(_, d)| d.len() as u64).sum::<u64>();
        out
    }

    fn join_allowed(&mut self, ip: IpAddr, now: Instant) -> bool {
        let e = self.join_rate.entry(ip).or_insert((now, 0));
        if now.duration_since(e.0) >= Duration::from_secs(10) {
            *e = (now, 0);
        }
        e.1 += 1;
        e.1 <= self.limits.joins_per_ip
    }

    fn on_join(&mut self, data: &[u8], from: Endpoint, ip: IpAddr, now: Instant) -> Vec<(Endpoint, Vec<u8>)> {
        if !self.join_allowed(ip, now) {
            self.stats.joins_rejected.fetch_add(1, Relaxed);
            return Vec::new();
        }
        let Ok(plain) = qt_transport::noise_n_open(&self.static_private, JOIN_PROLOGUE, &data[1..]) else {
            self.stats.joins_rejected.fetch_add(1, Relaxed);
            return Vec::new();
        };
        let plain = Zeroizing::new(plain);
        let Some((role, keys)) = parse_join_payload(&plain) else {
            self.stats.joins_rejected.fetch_add(1, Relaxed);
            return Vec::new();
        };
        if !self.rooms.contains_key(&keys.room_id) {
            let per_ip = self.rooms_by_ip.get(&ip).copied().unwrap_or(0);
            if self.rooms.len() >= self.limits.max_rooms || per_ip >= self.limits.rooms_per_ip {
                self.stats.joins_rejected.fetch_add(1, Relaxed);
                return Vec::new();
            }
            self.rooms.insert(
                keys.room_id,
                Room {
                    relay_key: Zeroizing::new(keys.relay_key),
                    slots: [None, None],
                    created: now,
                    last_activity: now,
                    creator_ip: ip,
                    video_allowed: true,
                },
            );
            *self.rooms_by_ip.entry(ip).or_insert(0) += 1;
            self.stats.rooms_opened.fetch_add(1, Relaxed);
        }
        let room = self.rooms.get_mut(&keys.room_id).expect("sala acabada de ver");
        if !ct_eq(&room.relay_key[..], &keys.relay_key) {
            self.stats.joins_rejected.fetch_add(1, Relaxed);
            return Vec::new();
        }
        self.stats.joins.fetch_add(1, Relaxed);
        room.last_activity = now;
        let me = idx(role);
        let newly_joined = room.slots[me].is_none();
        match room.slots[me].as_mut() {
            // Voltar a pedir (de outro endereco tambem: socket novo): o lado passa para la.
            Some(s) => s.ep = from,
            None => room.slots[me] = Some(Slot::new(from, self.limits.packets_per_sec, self.limits.video_bytes_per_sec, now)),
        }
        let other = room.slots[1 - me].as_ref().map(|s| s.ep);
        let mut out = vec![(from, join_ok(&keys, other.is_some()).to_vec())];
        // O outro estava sozinho: fica a saber que ja ha alguem (para comecar o audio).
        if let (Some(ep), true) = (other, newly_joined) {
            out.push((ep, join_ok(&keys, true).to_vec()));
        }
        out
    }

    /// "Que endereco ves?": so por UDP (pela ligacao WebSocket nao ha endereco util), so
    /// para salas abertas e com a etiqueta da sala; conta para o limite de pedidos por IP.
    fn on_addr(&mut self, data: &[u8], from: Endpoint, ip: IpAddr, now: Instant) -> Vec<(Endpoint, Vec<u8>)> {
        let Endpoint::Udp(seen) = from else { return Vec::new() };
        let Some(room_id) = addr_request_room(data) else { return Vec::new() };
        if !self.rooms.contains_key(&room_id) || !self.join_allowed(ip, now) {
            return Vec::new();
        }
        let room = self.rooms.get(&room_id).expect("sala acabada de ver");
        match addr_ok(&room.relay_key, data, &seen) {
            Some(ok) => {
                self.stats.addr_answered.fetch_add(1, Relaxed);
                vec![(from, ok.to_vec())]
            }
            None => Vec::new(),
        }
    }

    /// Video: so do endereco atual do lado (a mudanca de IP e pelo audio), dentro do debito
    /// do lado e do que a sala deixa. Quem envia recebe o aviso do modo de 5 em 5 s.
    fn on_video(&mut self, data: &[u8], from: Endpoint, now: Instant) -> Vec<(Endpoint, Vec<u8>)> {
        let Some(h) = parse_video_header(data) else { return Vec::new() };
        let Some(allowed) = self.rooms.get(&h.room_id).map(|r| r.video_allowed) else { return Vec::new() };
        let mode = self.video_mode(allowed);
        let rate = match mode {
            RelayVideo::Full => self.limits.video_bytes_per_sec,
            RelayVideo::Cap240 => self.limits.video_cap240_bytes_per_sec,
            RelayVideo::Denied => 0,
        } as f64;
        let room = self.rooms.get_mut(&h.room_id).expect("sala acabada de ver");
        if !relay_tag_valid(&room.relay_key, data) {
            return Vec::new();
        }
        let (me, other) = (idx(h.role), 1 - idx(h.role));
        let Some(slot) = room.slots[me].as_mut() else { return Vec::new() };
        if slot.ep != from {
            return Vec::new();
        }
        let mut out = Vec::new();
        let tell = slot.video_notice.is_none_or(|(t, m)| m != mode || now.duration_since(t) >= VIDEO_NOTICE_EVERY);
        if tell {
            slot.video_notice = Some((now, mode));
            out.push((from, relay_video_notice(&h.room_id, &room.relay_key, mode).to_vec()));
        }
        let elapsed = now.duration_since(slot.video_refill_at).as_secs_f64();
        slot.video_tokens = (slot.video_tokens + elapsed * rate).min(rate);
        slot.video_refill_at = now;
        if mode == RelayVideo::Denied || slot.video_tokens < data.len() as f64 {
            self.stats.video_dropped.fetch_add(1, Relaxed);
            return out;
        }
        slot.video_tokens -= data.len() as f64;
        room.last_activity = now;
        if let Some(peer) = room.slots[other].as_ref() {
            self.stats.video_forwarded.fetch_add(1, Relaxed);
            out.push((peer.ep, data.to_vec()));
        }
        out
    }

    /// Teto da conta (fichas anonimas, sessao propria): deixar ou nao passar video nesta sala.
    pub fn set_room_video(&mut self, room_id: &[u8; ROOM_ID_LEN], allowed: bool) {
        if let Some(r) = self.rooms.get_mut(room_id) {
            r.video_allowed = allowed;
        }
    }

    fn on_media(&mut self, data: &[u8], from: Endpoint, now: Instant) -> Vec<(Endpoint, Vec<u8>)> {
        let Some(h) = parse_header(data) else { return Vec::new() };
        let Some(room) = self.rooms.get_mut(&h.room_id) else { return Vec::new() };
        if !relay_tag_valid(&room.relay_key, data) {
            return Vec::new();
        }
        let pps = self.limits.packets_per_sec as f64;
        let (me, other) = (idx(h.role), 1 - idx(h.role));
        let Some(slot) = room.slots[me].as_mut() else { return Vec::new() };
        // Balde de fichas: `pps` por segundo, no maximo 1 s de reserva.
        let elapsed = now.duration_since(slot.refill_at).as_secs_f64();
        slot.tokens = (slot.tokens + elapsed * pps).min(pps);
        slot.refill_at = now;
        if slot.tokens < 1.0 {
            return Vec::new();
        }
        slot.tokens -= 1.0;
        if slot.ep != from {
            if slot.last_counter.is_some_and(|c| h.counter <= c) {
                return Vec::new();
            }
            slot.ep = from;
            self.stats.roamed.fetch_add(1, Relaxed);
        }
        slot.last_counter = Some(slot.last_counter.map_or(h.counter, |c| c.max(h.counter)));
        room.last_activity = now;
        match room.slots[other].as_ref() {
            Some(peer) => {
                self.stats.forwarded.fetch_add(1, Relaxed);
                vec![(peer.ep, data.to_vec())]
            }
            None => Vec::new(),
        }
    }

    /// Fecha salas abandonadas (e muda de mes). Chamar a cada poucos segundos.
    pub fn sweep(&mut self, now: Instant) -> usize {
        self.roll_month(current_month());
        let l = &self.limits;
        let mut closed = Vec::new();
        self.rooms.retain(|_, r| {
            let full = r.slots.iter().all(Option::is_some);
            let keep = now.duration_since(r.last_activity) < l.idle_timeout && (full || now.duration_since(r.created) < l.join_timeout);
            if !keep {
                closed.push(r.creator_ip);
            }
            keep
        });
        for ip in &closed {
            if let Some(n) = self.rooms_by_ip.get_mut(ip) {
                *n -= 1;
                if *n == 0 {
                    self.rooms_by_ip.remove(ip);
                }
            }
        }
        self.join_rate.retain(|_, (t, _)| now.duration_since(*t) < Duration::from_secs(10));
        closed.len()
    }

    /// Uma ligacao WebSocket fechou: esse lado sai das salas onde estava.
    pub fn endpoint_gone(&mut self, ep: Endpoint) {
        for r in self.rooms.values_mut() {
            for s in r.slots.iter_mut() {
                if s.as_ref().is_some_and(|x| x.ep == ep) {
                    *s = None;
                }
            }
        }
    }
}

// ------------------------------------------------------------------ rede

type WsSenders = Mutex<HashMap<u64, tokio::sync::mpsc::Sender<Vec<u8>>>>;

struct Shared {
    relay: Mutex<CallRelay>,
    udp: Arc<tokio::net::UdpSocket>,
    ws: WsSenders,
    next_ws: AtomicU64,
}

fn deliver(shared: &Shared, out: Vec<(Endpoint, Vec<u8>)>) {
    for (ep, data) in out {
        match ep {
            // `try_send_to`: nunca espera; se o buffer do socket estiver cheio, o pacote
            // perde-se (audio em tempo real: melhor perder do que atrasar).
            Endpoint::Udp(addr) => {
                let _ = shared.udp.try_send_to(&data, addr);
            }
            Endpoint::Ws(id) => {
                if let Some(tx) = shared.ws.lock().ok().and_then(|m| m.get(&id).cloned()) {
                    let _ = tx.try_send(data);
                }
            }
        }
    }
}

fn process(shared: &Shared, data: &[u8], from: Endpoint, ip: IpAddr) {
    let out = match shared.relay.lock() {
        Ok(mut r) => r.handle(data, from, ip, Instant::now()),
        Err(_) => return,
    };
    deliver(shared, out);
}

async fn udp_loop(shared: Arc<Shared>) {
    let mut buf = vec![0u8; 2048];
    loop {
        let Ok((n, addr)) = shared.udp.recv_from(&mut buf).await else {
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        };
        process(&shared, &buf[..n], Endpoint::Udp(addr), addr.ip());
    }
}

/// IP real de quem liga por WebSocket. Atras do HAProxy (ligacao vinda de localhost),
/// vale o `X-Forwarded-For` que o HAProxy poe (substitui o que o cliente mandar).
fn real_ip(peer: SocketAddr, forwarded: Option<&str>) -> IpAddr {
    if peer.ip().is_loopback() {
        if let Some(ip) = forwarded.and_then(|v| v.split(',').next()).and_then(|v| v.trim().parse().ok()) {
            return ip;
        }
    }
    peer.ip()
}

// O tipo do erro da callback e imposto pelo tungstenite.
#[allow(clippy::result_large_err)]
async fn ws_connection(shared: Arc<Shared>, tcp: tokio::net::TcpStream, peer: SocketAddr) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
    use tokio_tungstenite::tungstenite::Message;

    let mut forwarded: Option<String> = None;
    let cb = |req: &Request, resp: Response| {
        forwarded = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()).map(str::to_string);
        Ok(resp)
    };
    let cfg = tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
        max_message_size: Some(MAX_PACKET),
        max_frame_size: Some(MAX_PACKET),
        ..Default::default()
    };
    let accept = tokio_tungstenite::accept_hdr_async_with_config(tcp, cb, Some(cfg));
    let Ok(Ok(ws)) = tokio::time::timeout(Duration::from_secs(5), accept).await else { return };
    let ip = real_ip(peer, forwarded.as_deref());
    let id = shared.next_ws.fetch_add(1, Relaxed);
    let ep = Endpoint::Ws(id);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    if let Ok(mut m) = shared.ws.lock() {
        m.insert(id, tx);
    }
    let (mut sink, mut stream) = ws.split();
    // Sem nada durante o tempo de uma sala parada: a ligacao fecha.
    let idle = Duration::from_secs(40);
    loop {
        tokio::select! {
            msg = tokio::time::timeout(idle, stream.next()) => match msg {
                Ok(Some(Ok(Message::Binary(b)))) => process(&shared, &b, ep, ip),
                Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => {}
                _ => break,
            },
            Some(out) = rx.recv() => {
                if sink.send(Message::Binary(out)).await.is_err() {
                    break;
                }
            }
        }
    }
    if let Ok(mut m) = shared.ws.lock() {
        m.remove(&id);
    }
    if let Ok(mut r) = shared.relay.lock() {
        r.endpoint_gone(ep);
    }
}

async fn ws_loop(shared: Arc<Shared>, listener: tokio::net::TcpListener, max_ws: usize) {
    let slots = Arc::new(tokio::sync::Semaphore::new(max_ws));
    loop {
        let Ok((tcp, peer)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let Ok(permit) = slots.clone().try_acquire_owned() else { continue };
        let shared = shared.clone();
        tokio::spawn(async move {
            ws_connection(shared, tcp, peer).await;
            drop(permit);
        });
    }
}

/// Liga a logica aos sockets e devolve as contagens (para a linha por minuto).
/// `ws` e a reserva por TCP (atras do HAProxy, so em localhost).
pub fn serve(
    udp: tokio::net::UdpSocket,
    ws: Option<tokio::net::TcpListener>,
    static_private: &[u8],
    limits: CallLimits,
) -> Arc<CallStats> {
    let max_ws = limits.max_rooms * 2;
    let state_file = limits.state_file.clone();
    let mut relay = CallRelay::new(static_private, limits);
    if let Some(e) = state_file.as_deref().and_then(load_egress) {
        relay.set_egress(e);
    }
    let stats = relay.stats.clone();
    let shared = Arc::new(Shared { relay: Mutex::new(relay), udp: Arc::new(udp), ws: Mutex::new(HashMap::new()), next_ws: AtomicU64::new(1) });
    tokio::spawn(udp_loop(shared.clone()));
    if let Some(l) = ws {
        tokio::spawn(ws_loop(shared.clone(), l, max_ws));
    }
    let s = shared.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        let mut n = 0u32;
        loop {
            tick.tick().await;
            let egress = match s.relay.lock() {
                Ok(mut r) => {
                    r.sweep(Instant::now());
                    r.egress()
                }
                Err(_) => continue,
            };
            n += 1;
            // A conta do mes, de 10 em 10 s (num reinicio perde-se no maximo isso).
            if n.is_multiple_of(2) {
                if let Some(p) = state_file.as_deref() {
                    save_egress(p, egress);
                }
            }
        }
    });
    stats
}

/// Conta do mes gravada: "AAAAMM bytes".
pub fn load_egress(path: &std::path::Path) -> Option<Egress> {
    let s = std::fs::read_to_string(path).ok()?;
    let mut it = s.split_whitespace();
    Some(Egress { month: it.next()?.parse().ok()?, bytes: it.next()?.parse().ok()? })
}

/// Grava a conta do mes (ficheiro novo e troca, para nunca ficar a meio).
pub fn save_egress(path: &std::path::Path, e: Egress) {
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, format!("{} {}\n", e.month, e.bytes)).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Salas abertas agora (para a linha por minuto). `None` se o estado estiver bloqueado.
pub fn take_line(stats: &CallStats) -> Option<String> {
    let v = [
        stats.joins.swap(0, Relaxed),
        stats.joins_rejected.swap(0, Relaxed),
        stats.rooms_opened.swap(0, Relaxed),
        stats.forwarded.swap(0, Relaxed),
        stats.dropped.swap(0, Relaxed),
        stats.roamed.swap(0, Relaxed),
        stats.addr_answered.swap(0, Relaxed),
        stats.video_forwarded.swap(0, Relaxed),
        stats.video_dropped.swap(0, Relaxed),
    ];
    (v.iter().any(|x| *x > 0)).then(|| {
        format!(
            "chamadas: entradas={} recusadas={} salas_novas={} pacotes={} descartados={} mudancas_de_ip={} enderecos={} video={} video_descartado={}",
            v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8]
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qt_core::call::{join_payload, parse_join_ok, CallKeys, MediaSession, RoomKeys};

    struct Side {
        media: MediaSession,
        room: RoomKeys,
        role: Role,
    }

    fn sides(offer: u8) -> (Side, Side) {
        let (o, a, id) = ([offer; 32], [2u8; 32], [3u8; 16]);
        let mk = |role| Side {
            media: MediaSession::new(CallKeys::derive(&o, &a, &id, role), 64).unwrap(),
            room: RoomKeys::derive(&o, &id),
            role,
        };
        (mk(Role::Caller), mk(Role::Callee))
    }

    fn join(pubkey: &[u8], s: &Side) -> Vec<u8> {
        let mut p = vec![JOIN_KIND];
        p.extend(qt_transport::noise_n_seal(pubkey, JOIN_PROLOGUE, &join_payload(s.role, &s.room)).unwrap());
        p
    }

    fn ep(port: u16) -> Endpoint {
        Endpoint::Udp(SocketAddr::from(([10, 0, 0, 1], port)))
    }

    fn ip() -> IpAddr {
        IpAddr::from([10, 0, 0, 1])
    }

    fn setup(limits: CallLimits) -> (CallRelay, Vec<u8>) {
        let k = qt_transport::generate_static_keypair().unwrap();
        (CallRelay::new(&k.private, limits), k.public)
    }

    #[test]
    fn entrar_e_falar_pelos_dois_lados() {
        let (mut r, pk) = setup(CallLimits::default());
        let (mut a, mut b) = sides(1);
        let t = Instant::now();
        assert_eq!(join(&pk, &a).len(), JOIN_LEN);
        let out = r.handle(&join(&pk, &a), ep(1), ip(), t);
        assert_eq!(out.len(), 1);
        assert_eq!(parse_join_ok(&a.room, &out[0].1), Some(false));
        let out = r.handle(&join(&pk, &b), ep(2), ip(), t);
        // O Bob recebe "a Alice ja esta" e a Alice fica a saber que o Bob chegou.
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].0, parse_join_ok(&b.room, &out[0].1)), (ep(2), Some(true)));
        assert_eq!((out[1].0, parse_join_ok(&a.room, &out[1].1)), (ep(1), Some(true)));

        let p = a.media.seal(b"ola").unwrap();
        let out = r.handle(&p, ep(1), ip(), t);
        assert_eq!(out, vec![(ep(2), p.clone())]);
        assert_eq!(b.media.open(&out[0].1).unwrap().1, b"ola");
        let q = b.media.seal(b"ola!").unwrap();
        assert_eq!(r.handle(&q, ep(2), ip(), t), vec![(ep(1), q)]);
        assert_eq!(r.rooms(), 1);
    }

    #[test]
    fn sem_entrar_ou_com_etiqueta_errada_nada_passa() {
        let (mut r, pk) = setup(CallLimits::default());
        let (mut a, b) = sides(1);
        let t = Instant::now();
        // Audio antes de entrar: a sala nem existe.
        assert!(r.handle(&a.media.seal(b"x").unwrap(), ep(1), ip(), t).is_empty());
        r.handle(&join(&pk, &a), ep(1), ip(), t);
        r.handle(&join(&pk, &b), ep(2), ip(), t);
        let mut p = a.media.seal(b"x").unwrap();
        let n = p.len();
        p[n - 1] ^= 1;
        assert!(r.handle(&p, ep(1), ip(), t).is_empty());
        // Pedido de entrada para outra chave de relay: nao abre.
        let outra = qt_transport::generate_static_keypair().unwrap();
        let (c, _) = sides(9);
        assert!(r.handle(&join(&outra.public, &c), ep(3), ip(), t).is_empty());
        // Lixo e pacotes demasiado grandes.
        assert!(r.handle(&[0x20; 10], ep(3), ip(), t).is_empty());
        assert!(r.handle(&vec![0x10; MAX_PACKET + 1], ep(3), ip(), t).is_empty());
    }

    #[test]
    fn mudar_de_ip_so_com_pacote_novo() {
        let (mut r, pk) = setup(CallLimits::default());
        let (mut a, b) = sides(1);
        let t = Instant::now();
        r.handle(&join(&pk, &a), ep(1), ip(), t);
        r.handle(&join(&pk, &b), ep(2), ip(), t);
        let p0 = a.media.seal(b"0").unwrap();
        let p1 = a.media.seal(b"1").unwrap();
        r.handle(&p1, ep(1), ip(), t);
        // Alguem repete de outro sitio um pacote antigo (ou o mesmo): nao desvia a sala.
        assert!(r.handle(&p0, ep(66), ip(), t).is_empty());
        assert!(r.handle(&p1, ep(66), ip(), t).is_empty());
        // A Alice passa para o 4G: o pacote seguinte, com etiqueta valida, muda o endereco.
        let p2 = a.media.seal(b"2").unwrap();
        assert_eq!(r.handle(&p2, ep(7), ip(), t).len(), 1);
        // A resposta do Bob ja vai para o endereco novo.
        let (_, mut b2) = sides(1);
        let q = b2.media.seal(b"q").unwrap();
        assert_eq!(r.handle(&q, ep(2), ip(), t)[0].0, ep(7));
        // E o endereco antigo deixou de contar.
        let p_old = a.media.seal(b"3").unwrap();
        assert_eq!(r.handle(&p_old, ep(1), ip(), t).len(), 1, "pacote mais novo do endereco antigo volta a mudar");
        let _ = b;
    }

    #[test]
    fn limite_de_pacotes_por_segundo() {
        let limits = CallLimits { packets_per_sec: 10, ..Default::default() };
        let (mut r, pk) = setup(limits);
        let (mut a, b) = sides(1);
        let t = Instant::now();
        r.handle(&join(&pk, &a), ep(1), ip(), t);
        r.handle(&join(&pk, &b), ep(2), ip(), t);
        let passed = (0..30).filter(|_| !r.handle(&a.media.seal(b"x").unwrap(), ep(1), ip(), t).is_empty()).count();
        assert_eq!(passed, 10);
        let later = t + Duration::from_secs(1);
        assert!(!r.handle(&a.media.seal(b"x").unwrap(), ep(1), ip(), later).is_empty());
    }

    #[test]
    fn salas_por_ip_e_entradas_por_ip() {
        let limits = CallLimits { rooms_per_ip: 2, joins_per_ip: 5, ..Default::default() };
        let (mut r, pk) = setup(limits);
        let t = Instant::now();
        for i in 0..2u8 {
            let (a, _) = sides(10 + i);
            assert_eq!(r.handle(&join(&pk, &a), ep(1), ip(), t).len(), 1);
        }
        let (a, _) = sides(20);
        assert!(r.handle(&join(&pk, &a), ep(1), ip(), t).is_empty(), "terceira sala do mesmo IP");
        // Outro IP pode.
        let other = IpAddr::from([10, 0, 0, 2]);
        assert_eq!(r.handle(&join(&pk, &a), ep(9), other, t).len(), 1);
        // Mais de 5 pedidos em 10 s do mesmo IP: recusados (ja foram 3).
        let (x, _) = sides(10);
        assert_eq!(r.handle(&join(&pk, &x), ep(1), ip(), t).len(), 1);
        assert_eq!(r.handle(&join(&pk, &x), ep(1), ip(), t).len(), 1);
        assert!(r.handle(&join(&pk, &x), ep(1), ip(), t).is_empty());
        let later = t + Duration::from_secs(11);
        assert_eq!(r.handle(&join(&pk, &x), ep(1), ip(), later).len(), 1);
    }

    #[test]
    fn salas_abandonadas_fecham() {
        let (mut r, pk) = setup(CallLimits::default());
        let t = Instant::now();
        let (a, b) = sides(1);
        r.handle(&join(&pk, &a), ep(1), ip(), t);
        // So um lado: fecha aos 60 s mesmo que repita o pedido.
        r.handle(&join(&pk, &a), ep(1), ip(), t + Duration::from_secs(25));
        r.handle(&join(&pk, &a), ep(1), ip(), t + Duration::from_secs(50));
        assert_eq!(r.sweep(t + Duration::from_secs(55)), 0);
        assert_eq!(r.sweep(t + Duration::from_secs(61)), 1);
        // Dois lados e silencio: fecha aos 30 s sem pacotes.
        let t2 = t + Duration::from_secs(100);
        r.handle(&join(&pk, &a), ep(1), ip(), t2);
        r.handle(&join(&pk, &b), ep(2), ip(), t2);
        assert_eq!(r.sweep(t2 + Duration::from_secs(29)), 0);
        assert_eq!(r.sweep(t2 + Duration::from_secs(31)), 1);
        assert_eq!(r.rooms(), 0);
        assert!(r.rooms_by_ip.is_empty());
    }

    #[test]
    fn resposta_nunca_maior_que_o_pedido() {
        let (mut r, pk) = setup(CallLimits::default());
        let (a, _) = sides(1);
        let p = join(&pk, &a);
        let out = r.handle(&p, ep(1), ip(), Instant::now());
        assert!(out[0].1.len() < p.len());
    }

    #[test]
    fn endereco_visto_so_para_salas_abertas() {
        use qt_core::call::{addr_request, parse_addr_ok};
        let (mut r, pk) = setup(CallLimits::default());
        let (a, _) = sides(1);
        let t = Instant::now();
        let req = addr_request(&a.room, 9);
        // Sem sala: nada (o relay nao serve de STUN publico).
        assert!(r.handle(&req, ep(1), ip(), t).is_empty());
        r.handle(&join(&pk, &a), ep(1), ip(), t);
        let out = r.handle(&req, ep(5), ip(), t);
        assert_eq!(out.len(), 1);
        assert!(out[0].1.len() < req.len(), "resposta menor que o pedido");
        let Endpoint::Udp(seen) = ep(5) else { unreachable!() };
        assert_eq!((out[0].0, parse_addr_ok(&a.room, &out[0].1)), (ep(5), Some((9, seen))));
        // Etiqueta de outra sala ou pela ligacao WebSocket: nada.
        let (c, _) = sides(2);
        let mut forged = addr_request(&c.room, 9);
        forged[1..9].copy_from_slice(&a.room.room_id);
        assert!(r.handle(&forged, ep(5), ip(), t).is_empty());
        assert!(r.handle(&req, Endpoint::Ws(1), ip(), t).is_empty());
        // Nao muda o endereco do lado na sala (nao e audio).
        assert_eq!(r.rooms[&a.room.room_id].slots[0].as_ref().map(|s| s.ep), Some(ep(1)));
    }

    #[test]
    fn video_passa_dentro_do_debito_e_com_aviso() {
        let limits = CallLimits { video_bytes_per_sec: 10_000, ..Default::default() };
        let (mut r, pk) = setup(limits);
        let (a, b) = sides(1);
        let t = Instant::now();
        r.handle(&join(&pk, &a), ep(1), ip(), t);
        r.handle(&join(&pk, &b), ep(2), ip(), t);
        let mut va = a.media.video();
        let p = va.seal(&[7u8; 1000]).unwrap();
        assert_eq!(p.len(), 1065);
        // O primeiro leva o aviso "normal" para quem envia, e segue para o outro.
        let out = r.handle(&p, ep(1), ip(), t);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].0, qt_core::call::parse_relay_video_notice(&a.room, &out[0].1)), (ep(1), Some(RelayVideo::Full)));
        assert_eq!(out[1], (ep(2), p.clone()));
        // Debito: 10 000 bytes/s chegam para 9 pacotes deste tamanho.
        let passed = 1 + (0..20).filter(|_| r.handle(&va.seal(&[7u8; 1000]).unwrap(), ep(1), ip(), t).iter().any(|(e, _)| *e == ep(2))).count();
        assert_eq!(passed, 9);
        // Video de outro endereco (sem mudar de IP pelo audio): nada.
        assert!(r.handle(&va.seal(b"x").unwrap(), ep(9), ip(), t + Duration::from_secs(2)).is_empty());
        // O audio continua a passar com o video esgotado.
        let (mut a2, _) = sides(1);
        assert_eq!(r.handle(&a2.media.seal(b"voz").unwrap(), ep(1), ip(), t).len(), 1);
        // Pacotes de video com tamanho invalido: nada.
        assert!(r.handle(&p[..p.len() - 3], ep(1), ip(), t).is_empty());
    }

    #[test]
    fn travao_do_mes_e_teto_da_sala() {
        let limits = CallLimits { monthly_egress_bytes: 5_000, video_cap240_bytes_per_sec: 2_000, ..Default::default() };
        let (mut r, pk) = setup(limits);
        let (a, b) = sides(1);
        let t = Instant::now();
        r.handle(&join(&pk, &a), ep(1), ip(), t);
        r.handle(&join(&pk, &b), ep(2), ip(), t);
        let mut va = a.media.video();
        for _ in 0..5 {
            r.handle(&va.seal(&[1u8; 1000]).unwrap(), ep(1), ip(), t);
        }
        assert!(r.egress().bytes >= 5_000);
        // Passou do mes: aviso "240p" (logo, porque mudou) e balde mais pequeno.
        let out = r.handle(&va.seal(&[1u8; 1000]).unwrap(), ep(1), ip(), t);
        assert_eq!(qt_core::call::parse_relay_video_notice(&a.room, &out[0].1), Some(RelayVideo::Cap240));
        // Mes novo: volta ao normal.
        r.roll_month(299912);
        let out = r.handle(&va.seal(&[1u8; 100]).unwrap(), ep(1), ip(), t + Duration::from_secs(1));
        assert_eq!(qt_core::call::parse_relay_video_notice(&a.room, &out[0].1), Some(RelayVideo::Full));
        // Teto da conta (sala sem video): so o aviso, nada segue; o audio continua.
        r.set_room_video(&a.room.room_id, false);
        let out = r.handle(&va.seal(&[1u8; 100]).unwrap(), ep(1), ip(), t + Duration::from_secs(2));
        assert_eq!(out.len(), 1);
        assert_eq!(qt_core::call::parse_relay_video_notice(&a.room, &out[0].1), Some(RelayVideo::Denied));
        let (mut a2, _) = sides(1);
        let _ = a2.media.seal(b"0");
        assert_eq!(r.handle(&a2.media.seal(b"voz").unwrap(), ep(1), ip(), t).len(), 1);
    }

    #[test]
    fn mes_em_utc_e_conta_gravada() {
        assert_eq!(month_of_unix(1_792_713_600), 202610, "2026-10-23");
        assert_eq!(month_of_unix(1_709_164_800), 202402, "2024-02-29");
        assert_eq!(month_of_unix(1_798_761_599), 202612, "2026-12-31 23:59:59");
        assert_eq!(month_of_unix(1_798_761_600), 202701, "2027-01-01");
        assert_eq!(month_of_unix(0), 197001);
        let p = std::env::temp_dir().join(format!("qt_egress_{}", std::process::id()));
        save_egress(&p, Egress { month: 202610, bytes: 123 });
        assert_eq!(load_egress(&p), Some(Egress { month: 202610, bytes: 123 }));
        let _ = std::fs::remove_file(&p);
        let (mut r, _) = setup(CallLimits::default());
        let m = r.egress().month;
        r.set_egress(Egress { month: m, bytes: 77 });
        assert_eq!(r.egress().bytes, 77);
        r.set_egress(Egress { month: 190001, bytes: 99 });
        assert_eq!(r.egress().bytes, 0, "de outro mes: comeca do zero");
    }

    #[test]
    fn ip_real_atras_do_haproxy() {
        let local = SocketAddr::from(([127, 0, 0, 1], 5000));
        let fora = SocketAddr::from(([8, 8, 8, 8], 5000));
        assert_eq!(real_ip(local, Some("1.2.3.4")), IpAddr::from([1, 2, 3, 4]));
        assert_eq!(real_ip(local, None), local.ip());
        // De fora, o cabecalho nao conta (podia ser inventado).
        assert_eq!(real_ip(fora, Some("1.2.3.4")), fora.ip());
    }
}
