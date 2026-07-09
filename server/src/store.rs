//! Estado do servidor/relay: apenas duas coisas, ambas "cegas" quanto ao
//! conteudo real das conversas:
//!   1. Bundles de pre-keys publicas (necessarias para X3DH/PQXDH assincrono)
//!   2. Filas de mensagens cifradas, entregues e apagadas assim que o
//!      destinatario as recebe.
//!
//! Implementacao em memoria (HashMap + Mutex) - adequada para desenvolvimento
//! e testes. Antes de producao real, substituir por um backend persistente
//! (ex.: Redis para a fila, com TTL automatico - ver docs/protocol-spec.md
//! secao sobre retencao de mensagens).

use crate::protocol::UserId;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct Store {
    prekey_bundles: Mutex<HashMap<UserId, Vec<u8>>>,
    message_queues: Mutex<HashMap<UserId, Vec<(UserId, Vec<u8>)>>>,
}

/// Limite de mensagens em fila por utilizador, antes de comecar a recusar
/// novas mensagens - mitigacao simples de DoS (encher a fila de alguem
/// offline com lixo). Numero arbitrario para esta fase de desenvolvimento.
const MAX_QUEUED_MESSAGES_PER_USER: usize = 1000;

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_bundle(&self, user_id: UserId, bundle_bytes: Vec<u8>) {
        self.prekey_bundles.lock().unwrap().insert(user_id, bundle_bytes);
    }

    pub fn get_bundle(&self, user_id: &str) -> Option<Vec<u8>> {
        self.prekey_bundles.lock().unwrap().get(user_id).cloned()
    }

    /// Devolve `Err` se a fila do destinatario ja estiver no limite -
    /// o chamador deve informar o remetente que a entrega falhou, em vez
    /// de aceitar mensagens indefinidamente para uma conta inativa/inexistente.
    pub fn enqueue_message(
        &self,
        from: UserId,
        to: UserId,
        ciphertext: Vec<u8>,
    ) -> Result<(), &'static str> {
        let mut queues = self.message_queues.lock().unwrap();
        let queue = queues.entry(to).or_default();
        if queue.len() >= MAX_QUEUED_MESSAGES_PER_USER {
            return Err("fila do destinatario cheia");
        }
        queue.push((from, ciphertext));
        Ok(())
    }

    /// Retira e devolve todas as mensagens em fila para este utilizador
    /// (drena a fila - as mensagens nao ficam guardadas depois de entregues).
    pub fn drain_messages(&self, user_id: &str) -> Vec<(UserId, Vec<u8>)> {
        self.message_queues
            .lock()
            .unwrap()
            .remove(user_id)
            .unwrap_or_default()
    }
}
