//! Jitter buffer adaptativo: guarda os frames que chegam (fora de ordem, com atrasos
//! variaveis) e entrega um a cada 20 ms, ao ritmo do altifalante.
//!
//! COMO DECIDE (cada `pop`, a cada 20 ms):
//!   1. O frame seguinte chegou: toca-o.
//!   2. Nao chegou, mas ja ha um pacote mais a frente. Se o atraso atual ainda esta abaixo
//!      do alvo, espera por ele (PLC sem avancar, como em 3: o atraso sobe 20 ms de cada
//!      vez, ate ao alvo). Senao foi perdido: reconstroi-se com o DRED desse pacote
//!      (`Slot::Lost`), que traz o audio anterior comprimido. E o que deixa a copia pelo
//!      relay (ligacao direta em duplicado) chegar a tempo.
//!   3. Nao chegou e nao ha nada a frente (a rede ou o microfone do outro lado
//!      pararam): espera ate `MAX_HOLD` (200 ms, contados no relogio e nao em `pop`,
//!      porque o altifalante as vezes pede varios frames seguidos), preenchendo com PLC
//!      SEM avancar (`Slot::Conceal`). Cada espera aumenta o atraso em 20 ms; e assim
//!      que o atraso cresce quando a rede piora.
//!   4. Se continuar sem nada (rede cortada), desiste desse frame e avanca (PLC).
//!   5. Depois de desistir, se o que volta a chegar e mais antigo do que o frame
//!      seguinte (o outro telemovel parou e retomou atrasado em relacao ao tempo real),
//!      recomeca nesse pacote e corta o excesso ate ao atraso alvo. Sem isto, todos os
//!      pacotes seguintes chegavam "atrasados" e esse lado ficava mudo ate ao fim.
//!
//! O atraso alvo segue a variacao medida das chegadas (percentil 95 dos ultimos 3 s):
//! entre 40 e 200 ms. Os pacotes que chegam tarde demais tambem contam para a medida
//! (senao o alvo nunca aprendia com eles). Quando ha frames a mais do que o alvo durante meio segundo,
//! salta-se um frame (20 ms) para baixar o atraso.
//!
//! Nao depende do Opus nem da rede: os testes simulam o tempo.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

pub const FRAME: Duration = Duration::from_millis(20);
const FRAME_MS: f64 = 20.0;
/// Atraso alvo minimo e maximo (em frames de 20 ms).
pub const MIN_TARGET: u32 = 2;
pub const MAX_TARGET: u32 = 10;
/// Atraso no inicio da chamada, antes de haver medidas.
pub const START_TARGET: u32 = 3;
/// Quanto tempo se espera (com PLC) por um pacote atrasado antes de desistir. Cobre o
/// arranque lento do microfone e da rede do outro lado sem dar frames por perdidos.
pub const MAX_HOLD: Duration = Duration::from_millis(200);
/// Chegadas usadas para medir a variacao (3 s).
const WINDOW: usize = 150;
/// Frames a mais que o alvo, durante quantos `pop`, ate se saltar um frame.
const EXCESS_FRAMES: u32 = 2;
const EXCESS_POPS: u32 = 25;
/// Um salto maior que isto no contador (5 s) recomeca o buffer.
const RESYNC_FRAMES: u64 = 250;

/// O que tocar nos proximos 20 ms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    /// Ainda a encher (inicio da chamada): silencio.
    Waiting,
    /// O frame seguinte (Opus), para descodificar.
    Frame(Vec<u8>),
    /// Perdido, mas ja chegou um pacote mais a frente (`ahead_counter`): reconstruir
    /// com o DRED dele. `frames_back` = quantos frames antes desse pacote.
    Lost { ahead_counter: u64, ahead: Vec<u8>, frames_back: u32 },
    /// Nada para este frame: PLC.
    Conceal,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JitterStats {
    pub received: u64,
    /// Chegaram depois de se ter desistido deles (ja nao servem).
    pub late: u64,
    pub duplicates: u64,
    /// Perdidos na rede: faltou o frame mas chegou um pacote mais a frente
    /// (reconstruidos com o DRED desse pacote ou com PLC).
    pub lost: u64,
    /// Nao chegaram a tempo: nada chegou durante `MAX_HOLD` (a rede ou o microfone do
    /// outro lado pararam) e avancou-se sem eles (PLC).
    pub gave_up: u64,
    /// Vezes que se esperou por um pacote atrasado (cada uma acrescenta 20 ms).
    pub holds: u64,
    /// Frames saltados para baixar o atraso.
    pub skips: u64,
    /// Vezes que se recomecou depois de o outro lado parar e retomar atrasado.
    pub resyncs: u64,
    pub target_ms: u32,
    pub depth_ms: u32,
    /// Variacao das chegadas (percentil 95), em ms.
    pub jitter_ms: u32,
}

pub struct JitterBuffer {
    packets: BTreeMap<u64, Vec<u8>>,
    next: Option<u64>,
    start_at: Option<Instant>,
    newest: Option<u64>,
    target: u32,
    /// Desde quando se espera pelo frame `next` (sem nada a frente).
    hold_since: Option<Instant>,
    /// Desistiu-se e avanca-se as cegas (nada chegou durante `MAX_HOLD`).
    stalled: bool,
    /// Recomecou depois de uma paragem: no primeiro pop corta o excesso ate ao alvo.
    trim: bool,
    excess_pops: u32,
    origin: Option<(Instant, u64)>,
    transits: VecDeque<f64>,
    jitter_ms: f64,
    stats: JitterStats,
}

impl Default for JitterBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl JitterBuffer {
    pub fn new() -> Self {
        JitterBuffer {
            packets: BTreeMap::new(),
            next: None,
            start_at: None,
            newest: None,
            target: START_TARGET,
            hold_since: None,
            stalled: false,
            trim: false,
            excess_pops: 0,
            origin: None,
            transits: VecDeque::with_capacity(WINDOW + 1),
            jitter_ms: 0.0,
            stats: JitterStats::default(),
        }
    }

    fn restart(&mut self, counter: u64, now: Instant) {
        self.packets.clear();
        self.next = Some(counter);
        self.newest = None;
        self.start_at = Some(now + FRAME * self.target);
        self.hold_since = None;
        self.stalled = false;
        self.trim = false;
        self.excess_pops = 0;
        self.origin = None;
        self.transits.clear();
    }

    /// O outro lado parou e retomou atrasado em relacao ao tempo real: recomeca neste
    /// pacote (com o atraso alvo de antes) em vez de deitar fora tudo o que vier.
    fn resync(&mut self, counter: u64, now: Instant) {
        self.packets.clear();
        self.next = Some(counter);
        self.newest = None;
        self.start_at = Some(now + FRAME * self.target);
        self.hold_since = None;
        self.stalled = false;
        self.trim = true;
        self.excess_pops = 0;
        self.stats.resyncs += 1;
    }

    /// Chegou o frame com este contador (do pacote decifrado).
    pub fn push(&mut self, counter: u64, frame: Vec<u8>, now: Instant) {
        match self.next {
            None => self.restart(counter, now),
            Some(n) if counter > n + RESYNC_FRAMES => self.restart(counter, now),
            Some(n) if counter < n => {
                if !(self.stalled && self.packets.range(n..).next().is_none()) {
                    self.stats.late += 1;
                    // Chegou tarde: conta para a variacao, para o alvo subir e os seguintes
                    // como ele chegarem a tempo.
                    if !self.trim {
                        self.measure(counter, now);
                    }
                    return;
                }
                self.resync(counter, now);
            }
            _ => {}
        }
        if self.packets.contains_key(&counter) {
            self.stats.duplicates += 1;
            return;
        }
        // A recomecar (rajada do que estava retido): as chegadas nao dizem nada da rede.
        if !self.trim {
            self.measure(counter, now);
        }
        self.packets.insert(counter, frame);
        self.newest = Some(self.newest.map_or(counter, |x| x.max(counter)));
        self.stats.received += 1;
    }

    /// Variacao das chegadas: atraso de cada pacote em relacao ao relogio do envio
    /// (contador x 20 ms), menos o menor atraso visto. Percentil 95 da janela.
    fn measure(&mut self, counter: u64, now: Instant) {
        let (t0, c0) = *self.origin.get_or_insert((now, counter));
        let elapsed = now.duration_since(t0).as_secs_f64() * 1000.0;
        let transit = elapsed - (counter as i64 - c0 as i64) as f64 * FRAME_MS;
        self.transits.push_back(transit);
        if self.transits.len() > WINDOW {
            self.transits.pop_front();
        }
        let min = self.transits.iter().copied().fold(f64::INFINITY, f64::min);
        let mut dev: Vec<f64> = self.transits.iter().map(|t| t - min).collect();
        dev.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        self.jitter_ms = dev[(dev.len() * 95 / 100).min(dev.len() - 1)];
        // 1 frame pelo ritmo do audio + a variacao (com 5 ms de folga).
        let need = 1 + ((self.jitter_ms + 5.0) / FRAME_MS).ceil() as u32;
        self.target = need.clamp(MIN_TARGET, MAX_TARGET);
    }

    fn depth(&self) -> u32 {
        match (self.next, self.newest) {
            (Some(n), Some(w)) if w >= n => (w + 1 - n).min(u32::MAX as u64) as u32,
            _ => 0,
        }
    }

    /// O que tocar agora (chamar a cada 20 ms).
    pub fn pop(&mut self, now: Instant) -> Slot {
        let Some(mut next) = self.next else { return Slot::Waiting };
        if let Some(t) = self.start_at {
            if now < t {
                return Slot::Waiting;
            }
            self.start_at = None;
        }
        // Recomecou depois de uma paragem: fica so com o atraso alvo (os mais recentes),
        // de uma vez, e volta a medir as chegadas do zero.
        if self.trim {
            self.trim = false;
            while self.depth() > self.target {
                self.packets.remove(&next);
                next += 1;
                self.next = Some(next);
                self.stats.skips += 1;
            }
            self.origin = None;
            self.transits.clear();
        }
        // Atraso a mais durante meio segundo: salta um frame (so com os dois seguintes
        // presentes, para nao saltar por cima de uma perda).
        if self.depth() > self.target + EXCESS_FRAMES {
            self.excess_pops += 1;
        } else {
            self.excess_pops = 0;
        }
        if self.excess_pops >= EXCESS_POPS && self.packets.contains_key(&next) && self.packets.contains_key(&(next + 1)) {
            self.packets.remove(&next);
            next += 1;
            self.next = Some(next);
            self.stats.skips += 1;
            self.excess_pops = 0;
        }
        if let Some(f) = self.packets.remove(&next) {
            self.next = Some(next + 1);
            self.hold_since = None;
            self.stalled = false;
            return Slot::Frame(f);
        }
        // Falta este mas ha outro a frente: com menos atraso do que o alvo, espera por ele
        // (ate MAX_HOLD) em vez de o dar ja por perdido.
        if self.depth() < self.target && self.packets.range(next + 1..).next().is_some() {
            let since = *self.hold_since.get_or_insert(now);
            if now.saturating_duration_since(since) < MAX_HOLD {
                self.stats.holds += 1;
                return Slot::Conceal;
            }
        }
        if let Some((&m, p)) = self.packets.range(next + 1..).next() {
            let slot = Slot::Lost { ahead_counter: m, ahead: p.clone(), frames_back: (m - next) as u32 };
            self.next = Some(next + 1);
            self.hold_since = None;
            self.stalled = false;
            self.stats.lost += 1;
            return slot;
        }
        // Sem nada a frente: espera (PLC sem avancar) ate MAX_HOLD. Depois de desistir,
        // `hold_since` fica antigo e avanca-se a cada pop ate voltar a chegar alguma coisa.
        let since = *self.hold_since.get_or_insert(now);
        if now.saturating_duration_since(since) < MAX_HOLD {
            self.stats.holds += 1;
            return Slot::Conceal;
        }
        self.next = Some(next + 1);
        self.stalled = true;
        self.stats.gave_up += 1;
        Slot::Conceal
    }

    /// Contador do frame que esta a tocar (para sincronizar o video com o audio). `None`
    /// antes de comecar a tocar.
    pub fn playing(&self) -> Option<u64> {
        if self.start_at.is_some() {
            return None;
        }
        self.next.and_then(|n| n.checked_sub(1))
    }

    pub fn stats(&self) -> JitterStats {
        JitterStats {
            target_ms: self.target * FRAME_MS as u32,
            depth_ms: self.depth() * FRAME_MS as u32,
            jitter_ms: self.jitter_ms.round() as u32,
            ..self.stats.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(c: u64) -> Vec<u8> {
        vec![c as u8]
    }

    /// Relogio simulado: `ms` desde t0.
    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    /// Empurra `counters` cada um na sua hora (contador x 20 ms + `delay`).
    fn feed(j: &mut JitterBuffer, t0: Instant, counters: &[u64], delay: u64) {
        for &c in counters {
            j.push(c, f(c), at(t0, c * 20 + delay));
        }
    }

    /// Faz `n` pops a partir de `from_ms`, um a cada 20 ms.
    fn pops(j: &mut JitterBuffer, t0: Instant, from_ms: u64, n: u64) -> Vec<Slot> {
        (0..n).map(|i| j.pop(at(t0, from_ms + i * 20))).collect()
    }

    fn frames(slots: &[Slot]) -> Vec<u64> {
        slots
            .iter()
            .filter_map(|s| match s {
                Slot::Frame(v) => Some(v[0] as u64),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn espera_o_atraso_inicial_e_toca_por_ordem() {
        let t0 = Instant::now();
        let mut j = JitterBuffer::new();
        assert_eq!(j.pop(t0), Slot::Waiting);
        feed(&mut j, t0, &(0..10).collect::<Vec<_>>(), 0);
        // Antes de 60 ms (3 frames) ainda esta a encher.
        assert_eq!(j.pop(at(t0, 40)), Slot::Waiting);
        let s = pops(&mut j, t0, 60, 10);
        assert_eq!(frames(&s), (0..10).collect::<Vec<_>>());
        assert_eq!(j.stats().lost, 0);
    }

    #[test]
    fn fora_de_ordem_volta_a_ordem() {
        let t0 = Instant::now();
        let mut j = JitterBuffer::new();
        for (c, t) in [(0, 0), (2, 40), (1, 45), (3, 60), (5, 100), (4, 101)] {
            j.push(c, f(c), at(t0, t));
        }
        assert_eq!(frames(&pops(&mut j, t0, 100, 6)), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn perda_usa_o_pacote_a_frente_para_o_dred() {
        let t0 = Instant::now();
        let mut j = JitterBuffer::new();
        feed(&mut j, t0, &[0, 1, 5, 6], 0);
        let s = pops(&mut j, t0, 120, 6);
        assert_eq!(s[0], Slot::Frame(f(0)));
        assert_eq!(s[1], Slot::Frame(f(1)));
        assert_eq!(s[2], Slot::Lost { ahead_counter: 5, ahead: f(5), frames_back: 3 });
        assert_eq!(s[3], Slot::Lost { ahead_counter: 5, ahead: f(5), frames_back: 2 });
        assert_eq!(s[4], Slot::Lost { ahead_counter: 5, ahead: f(5), frames_back: 1 });
        assert_eq!(s[5], Slot::Frame(f(5)));
        assert_eq!(j.stats().lost, 3);
    }

    /// Ligacao direta em duplicado: um terco dos pacotes perdeu-se no direto e so chega a
    /// copia pelo relay, 100 ms depois. O alvo aprende com eles e o buffer espera: depois
    /// de aprender, quase nada se perde (antes perdia-se um terco).
    #[test]
    fn copia_atrasada_pelo_relay_chega_a_tempo() {
        let t0 = Instant::now();
        let mut j = JitterBuffer::new();
        let mut arrivals: Vec<(u64, u64)> = (0..300u64).map(|c| (c * 20 + 30 + if c % 3 == 1 { 100 } else { 0 }, c)).collect();
        arrivals.sort();
        let (mut ai, mut lost_after) = (0, 0u64);
        for ms in 0..6_400u64 {
            while ai < arrivals.len() && arrivals[ai].0 <= ms {
                let c = arrivals[ai].1;
                j.push(c, f(c), at(t0, ms));
                ai += 1;
            }
            if ms % 20 == 5 {
                let slot = j.pop(at(t0, ms));
                if ms >= 2_000 && matches!(slot, Slot::Lost { .. }) {
                    lost_after += 1;
                }
            }
        }
        let st = j.stats();
        assert!(st.target_ms >= 120, "o alvo devia cobrir os 100 ms: {st:?}");
        assert!(lost_after <= 2, "{lost_after} perdidos depois de aprender: {st:?}");
    }

    #[test]
    fn rede_atrasada_espera_sem_perder_o_pacote() {
        let t0 = Instant::now();
        let mut j = JitterBuffer::new();
        feed(&mut j, t0, &[0, 1], 0);
        let s = pops(&mut j, t0, 60, 4);
        assert_eq!(&s[..2], &[Slot::Frame(f(0)), Slot::Frame(f(1))]);
        // Sem nada a frente: PLC, mas fica a espera do 2.
        assert_eq!(&s[2..], &[Slot::Conceal, Slot::Conceal]);
        j.push(2, f(2), at(t0, 135));
        assert_eq!(j.pop(at(t0, 140)), Slot::Frame(f(2)));
        let st = j.stats();
        assert_eq!((st.holds, st.lost, st.late), (2, 0, 0));
    }

    #[test]
    fn rede_cortada_desiste_e_retoma() {
        let t0 = Instant::now();
        let mut j = JitterBuffer::new();
        feed(&mut j, t0, &[0], 0);
        // Toca o 0 (60 ms), espera 200 ms (10 pops, 80 a 260 ms) e desiste de 3 frames.
        let s = pops(&mut j, t0, 60, 1 + 10 + 3);
        assert!(s[1..].iter().all(|x| *x == Slot::Conceal));
        let st = j.stats();
        assert_eq!((st.holds, st.gave_up, st.lost), (10, 3, 0));
        // A rede volta (o outro lado continuou no tempo real): toca o que chega.
        j.push(4, f(4), at(t0, 330));
        assert_eq!(j.pop(at(t0, 340)), Slot::Frame(f(4)));
        assert_eq!(j.stats().resyncs, 0);
    }

    #[test]
    fn atrasado_com_a_chamada_a_correr_fica_de_fora() {
        let t0 = Instant::now();
        let mut j = JitterBuffer::new();
        feed(&mut j, t0, &[0, 1, 2, 3, 4], 0);
        pops(&mut j, t0, 60, 3);
        // Um repetido muito atrasado do 1, com a chamada a correr: ignorado.
        j.push(1, f(1), at(t0, 125));
        assert_eq!(j.pop(at(t0, 130)), Slot::Frame(f(3)));
        let st = j.stats();
        assert_eq!((st.late, st.resyncs), (1, 0));
    }

    /// O outro telemovel envia ao ritmo do tempo real, mas o microfone para `stall` ms e
    /// essas amostras perdem-se: o contador fica atrasado em relacao ao tempo real.
    /// Devolve (frames tocados nos ultimos 3 s de 6 s, estatisticas).
    fn microfone_parado(stall: u64) -> (usize, JitterStats) {
        let t0 = Instant::now();
        let mut j = JitterBuffer::new();
        let (mut c, mut tocados) = (0u64, 0);
        for ms in (0..6000u64).step_by(20) {
            if !(60..60 + stall).contains(&ms) {
                j.push(c, f(c), at(t0, ms + 30));
                c += 1;
            }
            if matches!(j.pop(at(t0, ms + 35)), Slot::Frame(_)) && ms >= 3000 {
                tocados += 1;
            }
        }
        (tocados, j.stats())
    }

    #[test]
    fn microfone_parado_sem_acumular_nao_deixa_o_lado_mudo() {
        for stall in [100, 200, 400, 600, 2000] {
            let (tocados, st) = microfone_parado(stall);
            assert!(tocados >= 148, "paragem de {stall} ms: so {tocados}/150 tocados, {st:?}");
            // Depois de recomecar, o atraso volta ao alvo.
            assert!(st.depth_ms <= st.target_ms + 40, "paragem de {stall} ms: {st:?}");
        }
    }

    #[test]
    fn rajada_retida_recomeca_com_o_atraso_alvo() {
        // O outro lado retem 600 ms (30 frames) e despeja tudo de uma vez.
        let t0 = Instant::now();
        let mut j = JitterBuffer::new();
        feed(&mut j, t0, &[0, 1, 2], 30);
        pops(&mut j, t0, 95, 3);
        // Espera 200 ms e desiste ate a rajada chegar.
        pops(&mut j, t0, 155, 25);
        assert!(j.stats().gave_up > 0);
        for c in 3..33 {
            j.push(c, f(c), at(t0, 660));
        }
        let s = pops(&mut j, t0, 665, 20);
        let st = j.stats();
        assert_eq!(st.resyncs, 1, "{st:?}");
        let fr = frames(&s);
        assert!(fr.windows(2).all(|w| w[1] == w[0] + 1), "{fr:?}");
        // Cortou o excesso: toca os mais recentes, nao os 600 ms retidos.
        assert!(fr[0] >= 33 - 1 - st.target_ms as u64 / 20, "{fr:?} {st:?}");
    }

    #[test]
    fn espera_conta_o_tempo_e_nao_os_pops() {
        // O altifalante as vezes pede varios frames de seguida (buffer a encher): isso
        // nao pode gastar a espera de 200 ms.
        let t0 = Instant::now();
        let mut j = JitterBuffer::new();
        feed(&mut j, t0, &[0, 1], 0);
        pops(&mut j, t0, 60, 2);
        for _ in 0..8 {
            assert_eq!(j.pop(at(t0, 100)), Slot::Conceal);
        }
        j.push(2, f(2), at(t0, 105));
        assert_eq!(j.pop(at(t0, 110)), Slot::Frame(f(2)));
        let st = j.stats();
        assert_eq!((st.gave_up, st.lost, st.late), (0, 0, 0));
    }

    #[test]
    fn microfone_lento_no_arranque_nao_perde_nada() {
        // O caso da chamada do Nuno (0.9.4): o outro telemovel envia 3 frames, o
        // microfone para ~200 ms e depois os frames retidos chegam de uma vez.
        let t0 = Instant::now();
        let mut j = JitterBuffer::new();
        let mut tocados = Vec::new();
        for ms in (0..2000u64).step_by(20) {
            for c in 0..100u64 {
                let enviado = c * 20;
                let chega = if c < 3 || enviado >= 260 { enviado + 30 } else { 290 };
                if chega >= ms && chega < ms + 20 {
                    j.push(c, f(c), at(t0, chega));
                }
            }
            if let Slot::Frame(v) = j.pop(at(t0, ms + 19)) {
                tocados.push(v[0] as u64);
            }
        }
        let st = j.stats();
        assert_eq!((st.lost, st.gave_up, st.late), (0, 0, 0), "{st:?}");
        assert!(tocados.windows(2).all(|w| w[1] > w[0]), "por ordem");
        assert!((0..=20).all(|c| tocados.contains(&c)), "o inicio toca todo: {tocados:?}");
        assert!(tocados.len() >= 85, "{}", tocados.len());
    }

    #[test]
    fn repetidos_ignorados() {
        let t0 = Instant::now();
        let mut j = JitterBuffer::new();
        j.push(0, f(0), t0);
        j.push(0, f(0), t0);
        assert_eq!(j.stats().duplicates, 1);
    }

    #[test]
    fn atraso_a_mais_e_reduzido() {
        let t0 = Instant::now();
        let mut j = JitterBuffer::new();
        // 2 s de audio chegam de uma vez (ex.: a rede despejou o que tinha retido).
        for c in 0..100 {
            j.push(c, f(c), at(t0, 0));
        }
        let s = pops(&mut j, t0, 60, 80);
        let fr = frames(&s);
        assert!(fr.windows(2).all(|w| w[1] > w[0]), "sempre por ordem");
        assert!(j.stats().skips >= 2, "saltou {} frames", j.stats().skips);
        assert_eq!(j.stats().lost, 0);
    }

    #[test]
    fn variacao_aumenta_o_alvo() {
        let t0 = Instant::now();
        let mut calmo = JitterBuffer::new();
        feed(&mut calmo, t0, &(0..100).collect::<Vec<_>>(), 30);
        assert_eq!(calmo.stats().target_ms, MIN_TARGET * 20);

        let mut agitado = JitterBuffer::new();
        for c in 0..100u64 {
            // metade dos pacotes chega 70 ms atrasada
            let extra = if c % 2 == 0 { 0 } else { 70 };
            agitado.push(c, f(c), at(t0, c * 20 + extra));
        }
        let st = agitado.stats();
        assert!(st.jitter_ms >= 60, "{st:?}");
        assert!(st.target_ms >= 100 && st.target_ms <= MAX_TARGET * 20, "{st:?}");
    }

    #[test]
    fn salto_grande_recomeca() {
        let t0 = Instant::now();
        let mut j = JitterBuffer::new();
        feed(&mut j, t0, &[0, 1, 2], 0);
        pops(&mut j, t0, 60, 2);
        j.push(10_000, f(16), at(t0, 200));
        assert_eq!(j.pop(at(t0, 220)), Slot::Waiting);
        assert_eq!(j.pop(at(t0, 300)), Slot::Frame(f(16)));
    }
}
