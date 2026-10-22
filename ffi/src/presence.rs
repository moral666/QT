//! Presenca: quem esta com a app aberta agora (online / offline).
//!
//! DESENHO ("so contactos, cifrado"): cada telemovel diz aos SEUS contactos, por
//! uma mensagem cifrada como as outras (tipo 9, dentro do Double Ratchet), quando
//! abre e fecha a app. O relay nao sabe o que e (ve um envelope 1 a 1) e nada
//! disto e guardado em disco: o estado fica so em memoria, neste processo.
//!
//!   * So para contactos (a lista que a app passa em `presence_configure`): de
//!     mais ninguem se aceita, e a mais ninguem se envia.
//!   * Interruptor: desligado, nao envia nem mostra (como noutras apps: quem
//!     esconde o seu estado tambem nao ve o dos outros). Ao desligar, quem me via
//!     online recebe logo "offline".
//!   * "online" vale `ONLINE_TTL_MS` a contar da hora em que foi enviado. Com a
//!     app aberta, a app manda um batimento (`presence_heartbeat`) a quem esta
//!     online; quem nao o receber passa a offline sozinho (telemovel sem rede,
//!     app morta). Um "online" antigo (ficou na fila do relay) nao conta.
//!   * Ao receber "online" de um contacto, respondo com o meu (se a app estiver
//!     aberta e nao lho tiver mandado ha pouco): assim quem acabou de abrir a app
//!     fica logo a saber quem esta online.
//!
//! FORMATO: payload v1, tipo 9, corpo [1 estado: 1 = online, 0 = offline]. O
//! `msg_id` e sempre o mesmo (`MSG_ID`): uma versao antiga da app, que nao
//! conhece o tipo 9, mostra o aviso "atualiza a app" uma so vez por contacto,
//! em vez de um aviso por cada abertura da app.

use std::collections::{HashMap, HashSet};

use crate::client::{QtClient, QtError};
use crate::history::{is_blocked_in, new_id, now_ms, ReceiveResult, PAYLOAD_V1};
use qt_storage as st;
use qt_storage::messages as hm;

pub(crate) const T_PRESENCE: u8 = 9;
/// Quanto tempo vale um "online" (a contar do envio).
pub const ONLINE_TTL_MS: i64 = 5 * 60_000;
/// Intervalo sugerido para o batimento com a app aberta (menor que o TTL).
pub const HEARTBEAT_MS: i64 = 4 * 60_000;
/// Nao repetir o "online" ao mesmo contacto antes disto (abrir e fechar a app seguido).
const ANNOUNCE_MIN_GAP_MS: i64 = 60_000;
/// Nao responder ao "online" de um contacto se lhe mandei o meu ha menos disto.
const REPLY_MIN_GAP_MS: i64 = 30_000;
const MSG_ID: [u8; 16] = *b"QT-PRESENCE-v1\0\0";

#[derive(Debug, Default)]
pub(crate) struct Presence {
    share: bool,
    allowed: HashSet<String>,
    visible: bool,
    /// contact_id -> ate quando esta online (ms).
    online_until: HashMap<String, i64>,
    /// contact_id -> (quando lhe mandei, o que mandei: true = online).
    last_sent: HashMap<String, (i64, bool)>,
}

/// Ate quando vale um "online" enviado a `sent_at` e recebido agora (`now`).
/// None: ja expirou. Um relogio adiantado do outro lado nao estica o prazo.
pub(crate) fn online_until(sent_at: i64, now: i64) -> Option<i64> {
    let until = sent_at.min(now) + ONLINE_TTL_MS;
    (until > now).then_some(until)
}

fn encode(online: bool, now: i64) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 1 + 16 + 8 + 1);
    out.push(PAYLOAD_V1);
    out.push(T_PRESENCE);
    out.extend_from_slice(&MSG_ID);
    out.extend_from_slice(&now.to_be_bytes());
    out.push(online as u8);
    out
}

/// Poe na outbox o meu estado para `cid` (so com sessao e nao bloqueado).
fn queue(client: &QtClient, c: &st::rusqlite::Connection, p: &mut Presence, cid: &str, online: bool) -> Result<bool, QtError> {
    if is_blocked_in(c, cid)? || st::load_session(c, cid)?.is_none() {
        return Ok(false);
    }
    let now = now_ms();
    let blob = client.seal_with(c, cid, &encode(online, now))?;
    hm::outbox_put(c, &new_id()?, cid, hm::OUTBOX_RECEIPT, &blob)?;
    p.last_sent.insert(cid.to_string(), (now, online));
    Ok(true)
}

/// Contactos que podem pensar que estou online (mandei-lhes "online" ha menos do TTL).
fn may_think_online(p: &Presence, now: i64) -> Vec<String> {
    p.last_sent.iter().filter(|(_, (t, on))| *on && now - t < ONLINE_TTL_MS).map(|(c, _)| c.clone()).collect()
}

fn offline_to_all(client: &QtClient, c: &st::rusqlite::Connection, p: &mut Presence) -> Result<u32, QtError> {
    let mut n = 0;
    for cid in may_think_online(p, now_ms()) {
        if queue(client, c, p, &cid, false)? {
            n += 1;
        }
    }
    Ok(n)
}

/// Tipo 9 recebido de `cid` (ja decifrado, dentro da transacao da rececao).
pub(crate) fn on_presence(
    client: &QtClient,
    c: &st::rusqlite::Connection,
    cid: &str,
    online: bool,
    sent_at: i64,
) -> Result<ReceiveResult, QtError> {
    let no_change = || ReceiveResult::NoChange { contact_id: cid.to_string() };
    let mut p = crate::client::relock(&client.presence);
    if !p.share || !p.allowed.contains(cid) {
        return Ok(no_change());
    }
    let now = now_ms();
    let was = p.online_until.get(cid).is_some_and(|u| *u > now);
    if !online {
        p.online_until.remove(cid);
        return Ok(if was { ReceiveResult::Updated { contact_id: cid.to_string() } } else { no_change() });
    }
    let Some(until) = online_until(sent_at, now) else { return Ok(no_change()) };
    p.online_until.insert(cid.to_string(), until);
    let recent = p.last_sent.get(cid).is_some_and(|(t, on)| *on && now - t < REPLY_MIN_GAP_MS);
    let replied = p.visible && !recent && queue(client, c, &mut p, cid, true)?;
    Ok(if !was || replied { ReceiveResult::Updated { contact_id: cid.to_string() } } else { no_change() })
}

#[uniffi::export]
impl QtClient {
    /// PRESENCA: liga/desliga (`share`) e diz quem sao os contactos (contact_ids):
    /// so eles recebem e mostram o meu estado. Chamar ao abrir e sempre que a lista
    /// de contactos ou o interruptor mudarem. Ao desligar, quem me via online
    /// recebe "offline" (devolve quantos envelopes ficaram na outbox).
    pub fn presence_configure(&self, share: bool, contact_ids: Vec<String>) -> Result<u32, QtError> {
        let c = self.lock()?;
        let mut p = crate::client::relock(&self.presence);
        let allowed: HashSet<String> = contact_ids.into_iter().collect();
        let tx = c.unchecked_transaction()?;
        let mut n = 0;
        if p.share && !share {
            n = offline_to_all(self, &tx, &mut p)?;
        }
        tx.commit()?;
        p.share = share;
        p.allowed = allowed;
        if !share {
            p.online_until.clear();
        } else {
            let allowed = p.allowed.clone();
            p.online_until.retain(|cid, _| allowed.contains(cid));
        }
        Ok(n)
    }

    /// A app abriu (true) ou saiu do ecra (false). Aberta: "online" a todos os
    /// contactos com sessao (nao repete a quem o recebeu ha menos de 1 min).
    /// Fechada: "offline" a quem pode pensar que estou online.
    pub fn presence_announce(&self, visible: bool) -> Result<u32, QtError> {
        let c = self.lock()?;
        let mut p = crate::client::relock(&self.presence);
        p.visible = visible;
        if !p.share {
            return Ok(0);
        }
        let tx = c.unchecked_transaction()?;
        let n = if visible {
            let now = now_ms();
            let mut targets: Vec<String> = p.allowed.iter().cloned().collect();
            targets.sort();
            let mut n = 0;
            for cid in targets {
                let fresh = p.last_sent.get(&cid).is_some_and(|(t, on)| *on && now - t < ANNOUNCE_MIN_GAP_MS);
                if !fresh && queue(self, &tx, &mut p, &cid, true)? {
                    n += 1;
                }
            }
            n
        } else {
            offline_to_all(self, &tx, &mut p)?
        };
        tx.commit()?;
        Ok(n)
    }

    /// Batimento com a app aberta (de `HEARTBEAT_MS` em `HEARTBEAT_MS`): "online"
    /// a quem esta online, para nao passar a offline do lado de la.
    pub fn presence_heartbeat(&self) -> Result<u32, QtError> {
        let c = self.lock()?;
        let mut p = crate::client::relock(&self.presence);
        if !p.share || !p.visible {
            return Ok(0);
        }
        let now = now_ms();
        let mut targets: Vec<String> = p.online_until.iter().filter(|(_, u)| **u > now).map(|(c, _)| c.clone()).collect();
        targets.sort();
        let tx = c.unchecked_transaction()?;
        let mut n = 0;
        for cid in targets {
            if queue(self, &tx, &mut p, &cid, true)? {
                n += 1;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// Contactos online agora (contact_ids). Vazio com a presenca desligada.
    pub fn presence_online(&self) -> Vec<String> {
        let p = crate::client::relock(&self.presence);
        if !p.share {
            return vec![];
        }
        let now = now_ms();
        let mut v: Vec<String> = p.online_until.iter().filter(|(_, u)| **u > now).map(|(c, _)| c.clone()).collect();
        v.sort();
        v
    }
}

/// Intervalo do batimento, em ms (para a app).
#[uniffi::export]
pub fn presence_heartbeat_ms() -> i64 {
    HEARTBEAT_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn online_vale_5_minutos_desde_o_envio() {
        let now = 1_000_000_000;
        assert_eq!(online_until(now, now), Some(now + ONLINE_TTL_MS));
        assert_eq!(online_until(now - 60_000, now), Some(now - 60_000 + ONLINE_TTL_MS));
        assert_eq!(online_until(now - ONLINE_TTL_MS, now), None, "ficou na fila do relay: ja nao conta");
        // Relogio do outro lado adiantado 1 hora: nao estica o prazo.
        assert_eq!(online_until(now + 3_600_000, now), Some(now + ONLINE_TTL_MS));
    }

    #[test]
    fn formato() {
        let p = encode(true, 7);
        assert_eq!((p[0], p[1], &p[2..18], p.len(), p[26]), (PAYLOAD_V1, T_PRESENCE, &MSG_ID[..], 27, 1));
    }
}
