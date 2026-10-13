//! Double Ratchet: cifra cada mensagem com uma chave unica e vai "rodando"
//! as chaves a cada troca, garantindo forward secrecy (comprometer uma chave
//! nao expoe mensagens passadas) e post-compromise security (a sessao
//! recupera seguranca apos algumas trocas, mesmo se um estado foi exposto).
//!
//! Limite de seguranca importante: MAX_SKIP define quantas mensagens "puladas"
//! (fora de ordem) ficam guardadas em memoria antes de serem descartadas -
//! isto evita que um atacante force acumulo indefinido de chaves (DoS de memoria).

use crate::primitives::{aead_decrypt, aead_encrypt, kdf_chain_key, kdf_root_key, CryptoError, DhKeyPair};
use zeroize::Zeroize;
use x25519_dalek::PublicKey;

/// Maximo de mensagens puladas por UM salto (uma cadeia, ou o fim da cadeia anterior).
const MAX_SKIP: usize = 1000;
/// Limite GLOBAL de chaves puladas guardadas (todas as cadeias). Ao exceder,
/// as mais antigas sao descartadas (memoria e disco limitados).
pub const MAX_SKIPPED_TOTAL: usize = 2000;

#[derive(Clone)]
pub struct RatchetState {
    root_key: [u8; 32],
    dh_send: DhKeyPair,
    dh_recv: Option<PublicKey>,
    sending_chain_key: Option<[u8; 32]>,
    receiving_chain_key: Option<[u8; 32]>,
    send_n: u32,
    recv_n: u32,
    /// Chaves puladas por ordem de insercao (a mais antiga primeiro).
    skipped_keys: Vec<(([u8; 32], u32), [u8; 32])>,
    /// PN: tamanho da cadeia de envio ANTERIOR (enviado no cabecalho).
    prev_send_n: u32,
    /// Dados associados da sessao (identidades), no AAD de cada mensagem.
    ad: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct EncryptedMessage {
    pub dh_public: PublicKey,
    /// PN: numero de mensagens da cadeia de envio anterior do remetente.
    pub pn: u32,
    pub n: u32,
    pub ciphertext: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum RatchetError {
    #[error("erro criptografico: {0}")]
    Crypto(#[from] CryptoError),
    #[error("numero de mensagens puladas excede o limite de seguranca ({0})")]
    TooManySkipped(usize),
    #[error("mensagem repetida ou demasiado antiga")]
    MessageTooOld,
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
        // Trailer (estados antigos nao o tem): [4 PN][2 len AD][AD]
        out.extend_from_slice(&self.prev_send_n.to_be_bytes());
        out.extend_from_slice(&(self.ad.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.ad);

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
        if skipped_count as usize > MAX_SKIPPED_TOTAL {
            return None;
        }
        let mut skipped_keys = Vec::new();
        for _ in 0..skipped_count {
            let pub_bytes: [u8; 32] = take(&mut cursor, 32)?.try_into().ok()?;
            let n = u32::from_be_bytes(take(&mut cursor, 4)?.try_into().ok()?);
            let key: [u8; 32] = take(&mut cursor, 32)?.try_into().ok()?;
            skipped_keys.push(((pub_bytes, n), key));
        }

        let (prev_send_n, ad) = if cursor == bytes.len() {
            (0, Vec::new())
        } else {
            let pn = u32::from_be_bytes(take(&mut cursor, 4)?.try_into().ok()?);
            let l = u16::from_be_bytes(take(&mut cursor, 2)?.try_into().ok()?) as usize;
            let ad = take(&mut cursor, l)?.to_vec();
            if cursor != bytes.len() {
                return None;
            }
            (pn, ad)
        };

        Some(Self {
            root_key,
            dh_send,
            dh_recv,
            sending_chain_key,
            receiving_chain_key,
            send_n,
            recv_n,
            skipped_keys,
            prev_send_n,
            ad,
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
            skipped_keys: Vec::new(),
            prev_send_n: 0,
            ad: Vec::new(),
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
            skipped_keys: Vec::new(),
            prev_send_n: 0,
            ad: Vec::new(),
        }
    }

    /// Numero de chaves puladas atualmente guardadas (sempre <= MAX_SKIPPED_TOTAL).
    pub fn skipped_key_count(&self) -> usize {
        self.skipped_keys.len()
    }

    /// Associa os dados da sessao (ver `identity::session_associated_data`).
    /// Tem de ser igual nos dois lados, ou nenhuma mensagem autentica.
    pub fn with_associated_data(mut self, ad: Vec<u8>) -> Self {
        self.ad = ad;
        self
    }

    fn dh_ratchet_step(&mut self, their_new_dh_public: PublicKey) {
        self.prev_send_n = self.send_n;
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
        let pn = self.prev_send_n;
        let aad = build_aad(&self.ad, &dh_public, pn, n);
        let ciphertext = aead_encrypt(&message_key, plaintext, &aad)?;

        Ok(EncryptedMessage { dh_public, pn, n, ciphertext })
    }

    /// Decifra uma mensagem. TRANSACIONAL: trabalha numa COPIA do estado e so
    /// a confirma se o AEAD autenticar. Qualquer falha (forjada, corrompida,
    /// repetida, demasiadas puladas...) deixa `self` exatamente como estava.
    pub fn decrypt(&mut self, msg: &EncryptedMessage) -> Result<Vec<u8>, RatchetError> {
        let mut work = self.clone();
        let plaintext = work.decrypt_unchecked(msg)?;
        *self = work; // so aqui o estado avanca
        Ok(plaintext)
    }

    /// Guarda as chaves das mensagens `recv_n..until` da cadeia atual.
    fn skip_message_keys(&mut self, dh_public: &[u8; 32], until: u32) -> Result<(), RatchetError> {
        if until <= self.recv_n {
            return Ok(());
        }
        if (until - self.recv_n) as usize > MAX_SKIP {
            return Err(RatchetError::TooManySkipped(MAX_SKIP));
        }
        let mut chain = self.receiving_chain_key.ok_or(RatchetError::Uninitialized)?;
        while self.recv_n < until {
            let (mk, next) = kdf_chain_key(&chain);
            self.skipped_keys.push(((*dh_public, self.recv_n), mk));
            chain = next;
            self.recv_n += 1;
        }
        self.receiving_chain_key = Some(chain);
        while self.skipped_keys.len() > MAX_SKIPPED_TOTAL {
            let (_, mut old) = self.skipped_keys.remove(0);
            old.zeroize();
        }
        Ok(())
    }

    fn decrypt_unchecked(&mut self, msg: &EncryptedMessage) -> Result<Vec<u8>, RatchetError> {
        let dh_bytes = msg.dh_public.to_bytes();

        // 1. Mensagem fora de ordem de uma cadeia ja conhecida?
        let aad = build_aad(&self.ad, &msg.dh_public, msg.pn, msg.n);
        if let Some(i) = self.skipped_keys.iter().position(|(k, _)| k.0 == dh_bytes && k.1 == msg.n) {
            let (_, mut mk) = self.skipped_keys.remove(i);
            let r = aead_decrypt(&mk, &msg.ciphertext, &aad);
            mk.zeroize();
            return Ok(r?);
        }

        // 2. Nova chave DH do par: guarda o resto da cadeia ANTERIOR (ate PN)
        //    e roda o DH ratchet.
        let needs_dh_step = match &self.dh_recv {
            Some(current) => current.as_bytes() != &dh_bytes,
            None => true,
        };
        if needs_dh_step {
            if let Some(old) = self.dh_recv {
                if self.receiving_chain_key.is_some() {
                    self.skip_message_keys(&old.to_bytes(), msg.pn)?;
                }
            }
            self.dh_ratchet_step(msg.dh_public);
        }

        // 3. Cadeia atual: mensagem ja consumida (e nao guardada) = repetida.
        if msg.n < self.recv_n {
            return Err(RatchetError::MessageTooOld);
        }
        self.skip_message_keys(&dh_bytes, msg.n)?;
        let chain = self.receiving_chain_key.ok_or(RatchetError::Uninitialized)?;
        let (mut mk, next) = kdf_chain_key(&chain);
        self.receiving_chain_key = Some(next);
        self.recv_n = self.recv_n.checked_add(1).ok_or(RatchetError::MessageTooOld)?;
        let r = aead_decrypt(&mk, &msg.ciphertext, &aad);
        mk.zeroize();
        Ok(r?)
    }
}

impl Drop for RatchetState {
    fn drop(&mut self) {
        self.root_key.zeroize();
        if let Some(k) = self.sending_chain_key.as_mut() { k.zeroize(); }
        if let Some(k) = self.receiving_chain_key.as_mut() { k.zeroize(); }
        for (_, k) in self.skipped_keys.iter_mut() { k.zeroize(); }
    }
}

/// Vincula o header (chave DH + numero da mensagem) criptograficamente ao
/// conteudo via AAD do AEAD - impede que um atacante na rede troque o
/// numero da mensagem ou a chave DH anunciada sem invalidar a autenticacao.
fn build_aad(ad: &[u8], dh_public: &PublicKey, pn: u32, n: u32) -> Vec<u8> {
    let mut aad = Vec::with_capacity(ad.len() + 40);
    aad.extend_from_slice(ad);
    aad.extend_from_slice(dh_public.as_bytes());
    aad.extend_from_slice(&pn.to_be_bytes());
    aad.extend_from_slice(&n.to_be_bytes());
    aad
}
