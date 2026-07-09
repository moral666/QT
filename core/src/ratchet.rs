//! Double Ratchet: cifra cada mensagem com uma chave unica e vai "rodando"
//! as chaves a cada troca, garantindo forward secrecy (comprometer uma chave
//! nao expoe mensagens passadas) e post-compromise security (a sessao
//! recupera seguranca apos algumas trocas, mesmo se um estado foi exposto).
//!
//! Limite de seguranca importante: MAX_SKIP define quantas mensagens "puladas"
//! (fora de ordem) ficam guardadas em memoria antes de serem descartadas -
//! isto evita que um atacante force acumulo indefinido de chaves (DoS de memoria).

use crate::primitives::{aead_decrypt, aead_encrypt, kdf_chain_key, kdf_root_key, CryptoError, DhKeyPair};
use std::collections::HashMap;
use x25519_dalek::PublicKey;

const MAX_SKIP: usize = 1000;

pub struct RatchetState {
    root_key: [u8; 32],
    dh_send: DhKeyPair,
    dh_recv: Option<PublicKey>,
    sending_chain_key: Option<[u8; 32]>,
    receiving_chain_key: Option<[u8; 32]>,
    send_n: u32,
    recv_n: u32,
    skipped_keys: HashMap<([u8; 32], u32), [u8; 32]>,
}

pub struct EncryptedMessage {
    pub dh_public: PublicKey,
    pub n: u32,
    pub ciphertext: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum RatchetError {
    #[error("erro criptografico: {0}")]
    Crypto(#[from] CryptoError),
    #[error("numero de mensagens puladas excede o limite de seguranca ({0})")]
    TooManySkipped(usize),
    #[error("ratchet nao inicializado corretamente (falta chain key)")]
    Uninitialized,
}

impl RatchetState {
    /// Serializa o estado completo da sessao para bytes, para persistencia
    /// local (ver storage/). O chamador e responsavel por cifrar estes
    /// bytes em repouso (ex.: SQLCipher) - esta funcao so faz a
    /// serializacao estrutural, nao adiciona cifra nenhuma por si so.
    ///
    /// Formato (tudo big-endian onde aplicavel):
    /// [32 root_key]
    /// [32 dh_send_private][32 dh_send_public]
    /// [1 has_dh_recv][32 dh_recv_public se has_dh_recv=1]
    /// [1 has_sending_chain][32 sending_chain_key se =1]
    /// [1 has_receiving_chain][32 receiving_chain_key se =1]
    /// [4 send_n][4 recv_n]
    /// [4 skipped_count][entradas: 32 pub + 4 n + 32 key, repetido skipped_count vezes]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.root_key);
        out.extend_from_slice(&self.dh_send.private.to_bytes());
        out.extend_from_slice(self.dh_send.public.as_bytes());

        match &self.dh_recv {
            Some(pk) => {
                out.push(1);
                out.extend_from_slice(pk.as_bytes());
            }
            None => out.push(0),
        }
        match &self.sending_chain_key {
            Some(k) => {
                out.push(1);
                out.extend_from_slice(k);
            }
            None => out.push(0),
        }
        match &self.receiving_chain_key {
            Some(k) => {
                out.push(1);
                out.extend_from_slice(k);
            }
            None => out.push(0),
        }

        out.extend_from_slice(&self.send_n.to_be_bytes());
        out.extend_from_slice(&self.recv_n.to_be_bytes());

        out.extend_from_slice(&(self.skipped_keys.len() as u32).to_be_bytes());
        for ((pub_bytes, n), key) in &self.skipped_keys {
            out.extend_from_slice(pub_bytes);
            out.extend_from_slice(&n.to_be_bytes());
            out.extend_from_slice(key);
        }

        out
    }

    /// Reconstroi um `RatchetState` a partir dos bytes produzidos por
    /// `to_bytes`. Devolve `None` se o formato for invalido/corrompido.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut cursor = 0usize;
        let take = |cursor: &mut usize, n: usize| -> Option<&[u8]> {
            let slice = bytes.get(*cursor..*cursor + n)?;
            *cursor += n;
            Some(slice)
        };

        let root_key: [u8; 32] = take(&mut cursor, 32)?.try_into().ok()?;
        let dh_send_private_bytes: [u8; 32] = take(&mut cursor, 32)?.try_into().ok()?;
        let dh_send_public_bytes: [u8; 32] = take(&mut cursor, 32)?.try_into().ok()?;

        let dh_send = DhKeyPair {
            private: x25519_dalek::StaticSecret::from(dh_send_private_bytes),
            public: PublicKey::from(dh_send_public_bytes),
        };

        let has_recv = *take(&mut cursor, 1)?.first()?;
        let dh_recv = if has_recv == 1 {
            let b: [u8; 32] = take(&mut cursor, 32)?.try_into().ok()?;
            Some(PublicKey::from(b))
        } else {
            None
        };

        let has_sending = *take(&mut cursor, 1)?.first()?;
        let sending_chain_key = if has_sending == 1 {
            Some(take(&mut cursor, 32)?.try_into().ok()?)
        } else {
            None
        };

        let has_receiving = *take(&mut cursor, 1)?.first()?;
        let receiving_chain_key = if has_receiving == 1 {
            Some(take(&mut cursor, 32)?.try_into().ok()?)
        } else {
            None
        };

        let send_n = u32::from_be_bytes(take(&mut cursor, 4)?.try_into().ok()?);
        let recv_n = u32::from_be_bytes(take(&mut cursor, 4)?.try_into().ok()?);

        let skipped_count = u32::from_be_bytes(take(&mut cursor, 4)?.try_into().ok()?);
        let mut skipped_keys = HashMap::new();
        for _ in 0..skipped_count {
            let pub_bytes: [u8; 32] = take(&mut cursor, 32)?.try_into().ok()?;
            let n = u32::from_be_bytes(take(&mut cursor, 4)?.try_into().ok()?);
            let key: [u8; 32] = take(&mut cursor, 32)?.try_into().ok()?;
            skipped_keys.insert((pub_bytes, n), key);
        }

        Some(Self {
            root_key,
            dh_send,
            dh_recv,
            sending_chain_key,
            receiving_chain_key,
            send_n,
            recv_n,
            skipped_keys,
        })
    }

    /// Inicializacao pelo lado que INICIOU o X3DH (Alice), ja conhecendo
    /// a signed pre-key publica de Bob como primeira chave DH remota.
    pub fn init_as_initiator(shared_secret: [u8; 32], their_dh_public: PublicKey) -> Self {
        let dh_send = DhKeyPair::generate();
        let dh_out = crate::primitives::dh(&dh_send.private, &their_dh_public);
        let (new_root, sending_chain) = kdf_root_key(&shared_secret, &dh_out);

        Self {
            root_key: new_root,
            dh_send,
            dh_recv: Some(their_dh_public),
            sending_chain_key: Some(sending_chain),
            receiving_chain_key: None,
            send_n: 0,
            recv_n: 0,
            skipped_keys: HashMap::new(),
        }
    }

    /// Inicializacao pelo lado que RESPONDEU ao X3DH (Bob). O par de chaves
    /// DH inicial de Bob e a propria signed pre-key ja publicada.
    pub fn init_as_responder(shared_secret: [u8; 32], my_signed_pre_key: DhKeyPair) -> Self {
        Self {
            root_key: shared_secret,
            dh_send: my_signed_pre_key,
            dh_recv: None,
            sending_chain_key: None,
            receiving_chain_key: None,
            send_n: 0,
            recv_n: 0,
            skipped_keys: HashMap::new(),
        }
    }

    fn dh_ratchet_step(&mut self, their_new_dh_public: PublicKey) {
        let dh_out = crate::primitives::dh(&self.dh_send.private, &their_new_dh_public);
        let (new_root, recv_chain) = kdf_root_key(&self.root_key, &dh_out);
        self.root_key = new_root;
        self.receiving_chain_key = Some(recv_chain);
        self.dh_recv = Some(their_new_dh_public);
        self.recv_n = 0;

        self.dh_send = DhKeyPair::generate();
        let dh_out2 = crate::primitives::dh(&self.dh_send.private, &their_new_dh_public);
        let (new_root2, send_chain) = kdf_root_key(&self.root_key, &dh_out2);
        self.root_key = new_root2;
        self.sending_chain_key = Some(send_chain);
        self.send_n = 0;
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<EncryptedMessage, RatchetError> {
        let chain = self.sending_chain_key.ok_or(RatchetError::Uninitialized)?;
        let (message_key, next_chain) = kdf_chain_key(&chain);
        self.sending_chain_key = Some(next_chain);

        let n = self.send_n;
        self.send_n += 1;

        let dh_public = self.dh_send.public;
        let aad = build_aad(&dh_public, n);
        let ciphertext = aead_encrypt(&message_key, plaintext, &aad);

        Ok(EncryptedMessage { dh_public, n, ciphertext })
    }

    pub fn decrypt(&mut self, msg: &EncryptedMessage) -> Result<Vec<u8>, RatchetError> {
        let needs_dh_step = match &self.dh_recv {
            Some(current) => current.as_bytes() != msg.dh_public.as_bytes(),
            None => true,
        };
        if needs_dh_step {
            self.dh_ratchet_step(msg.dh_public);
        }

        let skip_key = (msg.dh_public.to_bytes(), msg.n);
        let message_key = if let Some(mk) = self.skipped_keys.remove(&skip_key) {
            mk
        } else {
            let mut chain = self.receiving_chain_key.ok_or(RatchetError::Uninitialized)?;
            if (msg.n.saturating_sub(self.recv_n)) as usize > MAX_SKIP {
                return Err(RatchetError::TooManySkipped(MAX_SKIP));
            }
            while self.recv_n < msg.n {
                let (skipped_mk, next_chain) = kdf_chain_key(&chain);
                self.skipped_keys
                    .insert((msg.dh_public.to_bytes(), self.recv_n), skipped_mk);
                chain = next_chain;
                self.recv_n += 1;
            }
            let (mk, next_chain) = kdf_chain_key(&chain);
            self.receiving_chain_key = Some(next_chain);
            self.recv_n += 1;
            mk
        };

        let aad = build_aad(&msg.dh_public, msg.n);
        Ok(aead_decrypt(&message_key, &msg.ciphertext, &aad)?)
    }
}

/// Vincula o header (chave DH + numero da mensagem) criptograficamente ao
/// conteudo via AAD do AEAD - impede que um atacante na rede troque o
/// numero da mensagem ou a chave DH anunciada sem invalidar a autenticacao.
fn build_aad(dh_public: &PublicKey, n: u32) -> Vec<u8> {
    let mut aad = Vec::with_capacity(36);
    aad.extend_from_slice(dh_public.as_bytes());
    aad.extend_from_slice(&n.to_be_bytes());
    aad
}
