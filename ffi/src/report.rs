//! Denunciar um contacto (desde a 0.9.18). A pessoa escolhe as mensagens; a app junta-as
//! num JSON, cifra-o para a chave do operador (`qt_core::report`) e envia-o ao relay
//! (`SubmitReport`). O relay guarda bytes que nao consegue ler, sem o ID de quem denunciou.
//! So o operador (ferramenta `denuncias`, no computador do operador) abre a denuncia.
//!
//! Conteudo (JSON, antes de cifrar):
//! `{"v":1, "denunciado_public_id", "denunciado_relay_id", "motivo", "criada_em_ms",
//!   "mensagens":[{"de_denunciado", "enviada_em_ms", "texto", "anexo"}]}`.
//! Os anexos nao seguem: so a indicacao de que havia um (e o texto, ex. "📷 Foto").

use crate::client::{bad, hex, parse_public_id, QtClient, QtError};
use crate::history::{now_ms, ChatMessage};
use qt_core::report as qr;

/// Motivos aceites (a app mostra-os traduzidos).
pub const REASONS: [&str; 4] = ["spam", "assedio", "ilegal", "outro"];
/// Mensagens por denuncia, no maximo.
pub const MAX_MESSAGES: usize = 20;
/// Caracteres por mensagem, no maximo (o resto e cortado).
pub const MAX_CHARS: usize = 1000;
/// Mensagens recentes onde se procuram as escolhidas.
const LOOKBACK: u32 = 500;

/// A app consegue enviar denuncias (a chave do operador esta preenchida)?
#[uniffi::export]
pub fn reports_available() -> bool {
    qr::OPERATOR_REPORT_PUBLIC != [0u8; 32]
}

/// Monta o JSON da denuncia (sem cifrar). Separado para os testes.
pub(crate) fn report_json(
    public_id: &str,
    relay_id: &str,
    reason: &str,
    picked: &[ChatMessage],
) -> Result<Vec<u8>, QtError> {
    let mut msgs: Vec<serde_json::Value> = picked
        .iter()
        .map(|m| {
            let text: String = m.text.chars().take(MAX_CHARS).collect();
            serde_json::json!({
                "de_denunciado": !m.outgoing,
                "enviada_em_ms": m.sent_at_ms,
                "texto": text,
                "anexo": m.attachment.is_some(),
            })
        })
        .collect();
    loop {
        let body = serde_json::json!({
            "v": 1,
            "denunciado_public_id": public_id,
            "denunciado_relay_id": relay_id,
            "motivo": reason,
            "criada_em_ms": now_ms(),
            "mensagens": msgs,
        });
        let bytes = serde_json::to_vec(&body).map_err(|_| bad("denuncia"))?;
        if bytes.len() <= qr::MAX_PLAINTEXT {
            return Ok(bytes);
        }
        if msgs.is_empty() {
            return Err(bad("denuncia"));
        }
        msgs.remove(0); // nao cabe: deixa cair a mais antiga
    }
}

impl QtClient {
    /// A denuncia cifrada para `op_pub` (a app usa a chave fixa; os testes, outra).
    pub(crate) fn sealed_report(
        &self,
        public_id: &str,
        message_ids: &[String],
        reason: &str,
        op_pub: &[u8; 32],
    ) -> Result<Vec<u8>, QtError> {
        if !REASONS.contains(&reason) {
            return Err(bad("motivo"));
        }
        if message_ids.len() > MAX_MESSAGES {
            return Err(bad("mensagens"));
        }
        let (dh, sign) = parse_public_id(public_id)?;
        let mut picked = self.messages_page(hex(&dh), None, LOOKBACK)?;
        picked.retain(|m| message_ids.contains(&m.message_id));
        picked.sort_by_key(|m| m.seq);
        let json = report_json(public_id, &hex(&sign), reason, &picked)?;
        qr::seal_with(op_pub, &json).map_err(|e| match e {
            qr::ReportError::NoOperatorKey => bad("chave do operador"),
            _ => bad("denuncia"),
        })
    }
}

#[uniffi::export]
impl QtClient {
    /// Pedido `SubmitReport` para o relay, com a denuncia do contacto `public_id` ja cifrada
    /// para o operador. `message_ids`: as mensagens escolhidas da conversa (ate 20; podem
    /// ser 0). `reason`: um de `REASONS`. Resposta do relay: `Ack`; um relay antigo responde
    /// `Error` (pedido desconhecido). Depois a app bloqueia o contacto (`block_contact`).
    pub fn relay_request_report(
        &self,
        public_id: String,
        message_ids: Vec<String>,
        reason: String,
    ) -> Result<Vec<u8>, QtError> {
        let sealed = self.sealed_report(&public_id, &message_ids, &reason, &qr::OPERATOR_REPORT_PUBLIC)?;
        serde_json::to_vec(&serde_json::json!({"SubmitReport": {"sealed": sealed}})).map_err(|_| bad("denuncia"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::peer_bundle_from_relay_bytes;
    use std::sync::Arc;

    fn client(name: &str, key: u8) -> (Arc<QtClient>, crate::client::PublicIdentity) {
        let p = format!("/tmp/qt_report_{name}_{}.sqlite", std::process::id());
        let _ = std::fs::remove_file(&p);
        let c = QtClient::open(p, vec![key; 32]).unwrap();
        let id = c.create_identity().unwrap();
        (c, id)
    }

    #[test]
    fn denuncia_leva_so_as_mensagens_escolhidas_e_abre_com_a_chave_do_operador() {
        let (a, a_id) = client("alice", 0x11);
        let (b, b_id) = client("bob", 0x22);
        let a_cid = hex(&a_id.identity_public);
        let b_cid = hex(&b_id.identity_public);
        let bundle = peer_bundle_from_relay_bytes(b.publishable_bundle().unwrap()).unwrap();
        a.start_session(b_id.public_id.clone(), bundle).unwrap();
        for t in ["ola", "compra ja!!!", "ultima"] {
            a.send_text(b_cid.clone(), t.into()).unwrap();
        }
        for item in a.outbox().unwrap() {
            b.receive_sealed(item.blob.clone()).unwrap();
            a.outbox_sent(item.outbox_id).unwrap();
        }
        b.send_text(a_cid.clone(), "para com isso".into()).unwrap();

        let page = b.messages_page(a_cid.clone(), None, 50).unwrap();
        let ids: Vec<String> = page
            .iter()
            .filter(|m| m.text == "compra ja!!!" || m.text == "para com isso")
            .map(|m| m.message_id.clone())
            .collect();
        assert_eq!(ids.len(), 2);

        let seed = [7u8; 32];
        let op = qr::operator_report_public(&seed);
        let sealed = b.sealed_report(&a_id.public_id, &ids, "spam", &op).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&qr::open(&seed, &sealed).unwrap()).unwrap();
        assert_eq!(v["motivo"], "spam");
        assert_eq!(v["denunciado_public_id"], a_id.public_id.as_str());
        assert_eq!(v["denunciado_relay_id"], crate::client::relay_id_of_public_id(a_id.public_id.clone()).unwrap());
        let msgs = v["mensagens"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!((msgs[0]["texto"].as_str(), msgs[0]["de_denunciado"].as_bool()), (Some("compra ja!!!"), Some(true)));
        assert_eq!((msgs[1]["texto"].as_str(), msgs[1]["de_denunciado"].as_bool()), (Some("para com isso"), Some(false)));

        // Motivo desconhecido, mensagens a mais ou chave por preencher: recusado.
        assert!(b.sealed_report(&a_id.public_id, &ids, "outra coisa", &op).is_err());
        assert!(b.sealed_report(&a_id.public_id, &vec!["x".into(); 21], "spam", &op).is_err());
        assert!(b.sealed_report(&a_id.public_id, &ids, "spam", &[0u8; 32]).is_err());
        // Sem mensagens escolhidas tambem se pode denunciar.
        assert!(b.sealed_report(&a_id.public_id, &[], "assedio", &op).is_ok());
    }

    #[test]
    fn mensagens_compridas_sao_cortadas_e_a_denuncia_cabe_sempre() {
        let long = ChatMessage {
            seq: 1,
            message_id: "m".into(),
            contact_id: "c".into(),
            outgoing: false,
            text: "x".repeat(5000),
            sent_at_ms: 0,
            local_at_ms: 0,
            status: crate::history::MessageStatus::Received,
            attachment: None,
            reactions: vec![],
            sender: String::new(),
        };
        let many = vec![long; MAX_MESSAGES];
        let json = report_json("qt1:x", "y", "spam", &many).unwrap();
        assert!(json.len() <= qr::MAX_PLAINTEXT);
        let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert!(v["mensagens"].as_array().unwrap().iter().all(|m| m["texto"].as_str().unwrap().len() == MAX_CHARS));
    }
}
