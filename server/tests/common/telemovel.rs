//! Telemovel simulado para os testes de ponta a ponta: o nucleo verdadeiro (`QtClient`, SQLCipher
//! num ficheiro), a ligacao verdadeira ao relay (WebSocket + Noise + autenticacao) e os mesmos
//! passos da app Android (`MainViewModel`): `startLink` (registo do bundle), `flushOutbox`,
//! `pollOnce` (receber, guardar, confirmar), `restartSessions`, a copia de recuperacao e o
//! `restoreAccount`. O que a app faz no Kotlin esta aqui pela mesma ordem, para os testes
//! apanharem os erros que so aparecem com as duas partes juntas.
#![allow(dead_code)]

use futures_util::{SinkExt, StreamExt};
use qt_ffi::history::{ChatMessage, ReceiveResult};
use qt_ffi::recovery::BackupContact;
use qt_ffi::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Ligacao ao relay como a `RelayLink` da app.
pub struct Ligacao {
    ws: Ws,
    noise: Arc<NoiseSession>,
    /// Pedidos feitos ao relay por tipo (partilhado com o telemovel).
    pub contagem: Contagem,
}

pub type Contagem = Arc<std::sync::Mutex<std::collections::BTreeMap<String, u32>>>;

impl Ligacao {
    /// Liga-se e autentica-se (com a identidade do telemovel ou, sem ela, uma chave temporaria).
    pub async fn abrir(qt: &QtClient, url: &str) -> Result<Ligacao, String> {
        Self::abrir_com(qt, url, Default::default()).await
    }

    pub async fn abrir_com(qt: &QtClient, url: &str, contagem: Contagem) -> Result<Ligacao, String> {
        let noise = qt.relay_noise_session().map_err(|e| format!("{e:?}"))?;
        let (mut ws, _) = tokio_tungstenite::connect_async(url).await.map_err(|e| e.to_string())?;
        while !noise.is_finished() {
            let out = noise.write_step().map_err(|e| format!("{e:?}"))?;
            ws.send(Message::Binary(out)).await.map_err(|e| e.to_string())?;
            if !noise.is_finished() {
                match ws.next().await {
                    Some(Ok(Message::Binary(b))) => noise.read_step(b).map_err(|e| format!("{e:?}"))?,
                    other => return Err(format!("handshake: {other:?}")),
                }
            }
        }
        let mut l = Ligacao { ws, noise, contagem };
        let auth = qt.relay_auth_message(l.noise.handshake_hash().unwrap()).map_err(|e| format!("{e:?}"))?;
        match l.pedido(auth).await? {
            RelayResponse::Ack => Ok(l),
            outro => Err(format!("auth: {}", descrever(&outro))),
        }
    }

    pub async fn pedido(&mut self, req: Vec<u8>) -> Result<RelayResponse, String> {
        let tipo = String::from_utf8_lossy(&req[..req.len().min(40)]).split('"').nth(1).unwrap_or("?").to_string();
        *self.contagem.lock().unwrap().entry(tipo).or_default() += 1;
        let ct = self.noise.encrypt(req).map_err(|e| format!("{e:?}"))?;
        self.ws.send(Message::Binary(ct)).await.map_err(|e| e.to_string())?;
        loop {
            let m = tokio::time::timeout(Duration::from_secs(20), self.ws.next())
                .await
                .map_err(|_| "sem resposta do relay".to_string())?;
            match m {
                Some(Ok(Message::Binary(b))) => {
                    let pt = self.noise.decrypt(b).map_err(|e| format!("{e:?}"))?;
                    return relay_parse_response(pt).map_err(|e| format!("{e:?}"));
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                other => return Err(format!("ligacao: {other:?}")),
            }
        }
    }
}

pub fn descrever(r: &RelayResponse) -> String {
    match r {
        RelayResponse::Error { reason } => format!("Error({reason})"),
        RelayResponse::Ack => "Ack".into(),
        outro => format!("{:?}", std::mem::discriminant(outro)),
    }
}

#[derive(Clone, Debug)]
pub struct Contacto {
    pub nome: String,
    pub public_id: String,
    pub relay_id: String,
}

impl Contacto {
    /// Como `Contact.contactId` no Kotlin: a chave de identidade dentro do Public ID.
    pub fn contact_id(&self) -> String {
        self.public_id[4..68].to_string()
    }
}

/// Como a app reage a um "demasiados pedidos" ao enviar a outbox.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Outbox {
    /// Ate a 1.0.3: segue para o item seguinte (e volta a tentar tudo no proximo ciclo).
    Insiste,
    /// Desde a 1.0.4: para e deixa o resto para o ciclo seguinte.
    ParaNoLimite,
}

pub struct Telemovel {
    pub nome: String,
    dir: PathBuf,
    key: Vec<u8>,
    pub qt: Arc<QtClient>,
    pub contactos: Vec<Contacto>,
    pub url: String,
    /// Lista de relays da build (nome, url), o primeiro e o principal. Vazia: so `url`.
    pub relays: Vec<(String, String)>,
    pub link: Option<Ligacao>,
    pub outbox: Outbox,
    /// Respostas do relay que nao foram Ack aos envios da outbox (motivo -> vezes).
    pub recusas: std::collections::BTreeMap<String, u32>,
    /// Erros de receber (motivo -> vezes), como os `QtException` que a app engole.
    pub erros_receber: std::collections::BTreeMap<String, u32>,
    /// Pedidos feitos ao relay por tipo, desde que o telemovel foi instalado.
    pub pedidos: Contagem,
    seq: u32,
}

fn pasta(nome: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("qt-tel-{}-{}-{n}", std::process::id(), nome));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

impl Telemovel {
    /// Telemovel acabado de instalar (sem conta), ligado ao relay `url`.
    pub fn novo(nome: &str, url: &str) -> Telemovel {
        let dir = pasta(nome);
        let key: Vec<u8> = (0..32).map(|i| (i as u8).wrapping_mul(31) ^ nome.len() as u8 ^ 0x5A).collect();
        let qt = QtClient::open(dir.join("qt.db").to_string_lossy().into_owned(), key.clone()).unwrap();
        Telemovel {
            nome: nome.into(),
            dir,
            key,
            qt,
            contactos: vec![],
            url: url.into(),
            relays: vec![],
            link: None,
            outbox: Outbox::ParaNoLimite,
            recusas: Default::default(),
            erros_receber: Default::default(),
            pedidos: Default::default(),
            seq: 0,
        }
    }

    pub fn public_id(&self) -> String {
        self.qt.export_public_identity().unwrap().public_id
    }

    /// Quantos pedidos deste tipo (ex.: "FetchPreKeyBundle", "SendMessage") fez ao relay.
    pub fn pedidos_de(&self, tipo: &str) -> u32 {
        self.pedidos.lock().unwrap().get(tipo).copied().unwrap_or(0)
    }

    pub fn como_contacto(&self, nome: &str) -> Contacto {
        let pid = self.public_id();
        Contacto { nome: nome.into(), relay_id: relay_id_of_public_id(pid.clone()).unwrap(), public_id: pid }
    }

    /// `createIdentity` + `startLink`.
    pub async fn criar_conta(&mut self) {
        self.qt.create_identity().unwrap();
        self.ligar().await.unwrap();
    }

    /// `startLink`: liga, autentica e publica o bundle. Devolve o motivo se o relay recusar.
    pub async fn ligar(&mut self) -> Result<(), String> {
        let mut tentados: Vec<String> = vec![];
        for _ in 0..4 {
            self.link = None;
            let mut l = Ligacao::abrir_com(&self.qt, &self.url, self.pedidos.clone()).await?;
            let req = self.qt.relay_request_register_bundle(self.qt.publishable_bundle().unwrap()).unwrap();
            let r = l.pedido(req).await?;
            self.link = Some(l);
            let reason = match r {
                RelayResponse::Ack => return Ok(()),
                RelayResponse::Error { reason } => reason,
                outro => return Err(descrever(&outro)),
            };
            // `moveRelay`: a conta vive noutro relay, ou este nao aceita contas novas.
            let atual = self.relays.iter().find(|(_, u)| *u == self.url).map(|(n, _)| n.clone()).unwrap_or_default();
            let alvo = if let Some(n) = relay_elsewhere_name(reason.clone()) {
                self.relays.iter().find(|(x, _)| *x == n).cloned()
            } else if reason == relay_no_new_accounts_reason() {
                tentados.push(atual.clone());
                self.relays.iter().find(|(x, _)| !tentados.contains(x)).cloned()
            } else {
                None
            };
            match alvo {
                Some((n, u)) if n != atual => {
                    self.url = u;
                    let _ = self.qt.call_set_relay(Some(n));
                }
                _ => return Err(format!("Error({reason})")),
            }
        }
        Err("demasiadas mudancas de relay".into())
    }

    /// Reinicia a app (fecha e reabre a base de dados, liga de novo).
    pub async fn reiniciar_app(&mut self) {
        self.link = None;
        let path = self.dir.join("qt.db").to_string_lossy().into_owned();
        self.qt = QtClient::open(path, self.key.clone()).unwrap();
        self.ligar().await.unwrap();
    }

    /// Sem rede (a ligacao cai; a app continua a guardar na outbox).
    pub fn sem_rede(&mut self) {
        self.link = None;
    }

    /// `addContact`.
    pub fn adicionar(&mut self, c: Contacto) {
        let cid = c.contact_id();
        self.contactos.retain(|x| x.contact_id() != cid);
        self.contactos.push(c);
        let _ = self.qt.dismiss_intro(cid);
        self.sincronizar_contactos();
    }

    /// `syncCallContacts` (so tocam contactos guardados) e o resto que a app faz com a lista.
    pub fn sincronizar_contactos(&self) {
        let ids: Vec<String> = self.contactos.iter().map(|c| c.contact_id()).collect();
        self.qt.call_contacts_sync(ids).unwrap();
    }

    /// `acceptInvite`.
    pub fn aceitar_convite(&mut self, contact_id: &str, nome: &str) {
        let pid = self.qt.pending_intro(contact_id.into()).unwrap().expect("convite");
        self.adicionar(Contacto { nome: nome.into(), relay_id: relay_id_of_public_id(pid.clone()).unwrap(), public_id: pid });
    }

    fn contacto(&self, cid: &str) -> Option<&Contacto> {
        self.contactos.iter().find(|c| c.contact_id() == cid)
    }

    /// `ensureSession`.
    pub async fn garantir_sessao(&mut self, cid: &str) -> Result<(), String> {
        if self.qt.has_session(cid.into()).unwrap() {
            return Ok(());
        }
        let c = self.contacto(cid).cloned().ok_or("nao e contacto")?;
        let l = self.link.as_mut().ok_or("sem ligacao")?;
        match l.pedido(self.qt.relay_request_fetch_bundle(c.relay_id.clone()).unwrap()).await? {
            RelayResponse::Bundle { bytes } => {
                let b = peer_bundle_from_relay_bytes(bytes).map_err(|e| format!("{e:?}"))?;
                self.qt.start_session(c.public_id.clone(), b).map_err(|e| format!("{e:?}"))?;
                Ok(())
            }
            RelayResponse::BundleNotFound => Err("contacto sem chaves".into()),
            outro => Err(descrever(&outro)),
        }
    }

    /// `send`: sessao, guardar + outbox, `flushOutbox`.
    pub async fn enviar(&mut self, cid: &str, texto: &str) -> Result<(), String> {
        self.garantir_sessao(cid).await?;
        self.qt.send_text(cid.into(), texto.into()).map_err(|e| format!("{e:?}"))?;
        self.flush().await;
        Ok(())
    }

    /// `flushOutbox`, com o comportamento da versao escolhida em [Self::outbox].
    pub async fn flush(&mut self) -> usize {
        let Some(l) = self.link.as_mut() else { return 0 };
        let mut enviados = 0;
        let items = self.qt.outbox().unwrap();
        // Membros dos grupos que nao estao nos contactos: o endereco vem do Public ID (como na app).
        let membros: std::collections::HashMap<String, String> =
            self.qt.groups().unwrap().into_iter().flat_map(|g| g.members).map(|m| (m.contact_id, m.public_id)).collect();
        for item in items {
            let relay_id = match item
                .relay_id
                .clone()
                .or_else(|| self.contactos.iter().find(|c| c.contact_id() == item.contact_id).map(|c| c.relay_id.clone()))
                .or_else(|| membros.get(&item.contact_id).and_then(|p| relay_id_of_public_id(p.clone()).ok()))
            {
                Some(r) => r,
                None => continue,
            };
            let req = self.qt.relay_request_send_item(relay_id, item.clone()).unwrap();
            let r = match l.pedido(req).await {
                Ok(r) => r,
                Err(_) => break, // sem rede: fica para depois
            };
            match &r {
                RelayResponse::Ack => {
                    self.qt.outbox_sent(item.outbox_id.clone()).unwrap();
                    enviados += 1;
                }
                RelayResponse::Error { reason } => {
                    *self.recusas.entry(reason.clone()).or_default() += 1;
                    if *reason == relay_recipient_deleted_reason() {
                        self.qt.outbox_drop(item.outbox_id.clone()).unwrap();
                    } else if *reason == "demasiados pedidos, tenta daqui a pouco" && self.outbox == Outbox::ParaNoLimite {
                        break;
                    }
                }
                _ => {}
            }
        }
        enviados
    }

    /// `pollOnce` + `restartSessions`: envia, recebe todas as paginas, guarda, confirma, recomeca
    /// sessoes terminadas, recibos de entrega e envia outra vez. Devolve quantas mensagens tratou.
    pub async fn ciclo(&mut self) -> usize {
        if self.link.is_none() {
            return 0;
        }
        self.flush().await;
        let mut total = 0;
        let mut recebeu = false;
        for _ in 0..20 {
            let l = self.link.as_mut().unwrap();
            let msgs = match l.pedido(self.qt.relay_request_fetch().unwrap()).await {
                Ok(RelayResponse::Messages { messages }) => messages,
                _ => break,
            };
            if msgs.is_empty() {
                break;
            }
            let mut ack = vec![];
            for m in msgs {
                match self.qt.receive_sealed(m.ciphertext.clone()) {
                    Ok(r) => {
                        if !matches!(r, ReceiveResult::NoChange { .. }) {
                            recebeu = true;
                        }
                        ack.push(m.id);
                    }
                    Err(QtError::Storage) => {
                        *self.erros_receber.entry("Storage".into()).or_default() += 1;
                    }
                    Err(e) => {
                        *self.erros_receber.entry(format!("{e:?}").chars().take(40).collect()).or_default() += 1;
                        ack.push(m.id);
                    }
                }
                total += 1;
            }
            let l = self.link.as_mut().unwrap();
            let _ = l.pedido(self.qt.relay_request_ack(ack).unwrap()).await;
        }
        self.recomecar_sessoes().await;
        if recebeu {
            self.qt.prepare_delivery_receipts().unwrap();
            self.flush().await;
        }
        total
    }

    /// `restartSessions`.
    pub async fn recomecar_sessoes(&mut self) {
        let ids = self.qt.sessions_to_restart().unwrap();
        let mut abriu = false;
        for cid in ids {
            if self.contacto(&cid).is_none() || self.qt.is_blocked(cid.clone()).unwrap() {
                continue;
            }
            if self.garantir_sessao(&cid).await.is_ok() {
                abriu = true;
            }
        }
        if abriu {
            self.flush().await;
        }
    }

    /// Textos da conversa (por ordem), sem os meus.
    pub fn recebidas(&self, cid: &str) -> Vec<String> {
        self.conversa(cid).into_iter().filter(|m| !m.outgoing).map(|m| m.text).collect()
    }

    pub fn minhas(&self, cid: &str) -> Vec<ChatMessage> {
        self.conversa(cid).into_iter().filter(|m| m.outgoing).collect()
    }

    /// A conversa toda, da mais antiga para a mais recente.
    pub fn conversa(&self, cid: &str) -> Vec<ChatMessage> {
        let mut v = self.qt.messages_page(cid.into(), None, 1000).unwrap();
        v.sort_by_key(|m| m.seq);
        v
    }


    // ------------------------------------------------------------ grupos

    /// `ensureSessions`.
    pub async fn sessoes_com(&mut self, public_ids: Vec<String>) {
        for pid in public_ids {
            let cid = pid[4..68].to_string();
            if self.qt.is_blocked(cid.clone()).unwrap() || self.qt.has_session(cid).unwrap() {
                continue;
            }
            let l = self.link.as_mut().unwrap();
            if let Ok(RelayResponse::Bundle { bytes }) = l.pedido(self.qt.relay_request_fetch_bundle(relay_id_of_public_id(pid.clone()).unwrap()).unwrap()).await {
                let _ = self.qt.start_session(pid, peer_bundle_from_relay_bytes(bytes).unwrap());
            }
        }
    }

    /// `createGroup`: devolve o id do grupo.
    pub async fn criar_grupo(&mut self, nome: &str, membros: &[Contacto]) -> String {
        let pids: Vec<String> = membros.iter().map(|c| c.public_id.clone()).collect();
        self.sessoes_com(pids.clone()).await;
        let g = self.qt.group_create(nome.into(), pids).unwrap();
        self.flush().await;
        g.group_id
    }

    /// `sendGroup`.
    pub async fn enviar_grupo(&mut self, gid: &str, texto: &str) {
        let need = self.qt.group_members_needing_session(gid.into()).unwrap();
        self.sessoes_com(need).await;
        self.qt.group_send_text(gid.into(), texto.into()).unwrap();
        self.flush().await;
    }

    pub fn recebidas_grupo(&self, gid: &str) -> Vec<String> {
        let mut v = self.qt.group_messages_page(gid.into(), None, 1000).unwrap();
        v.sort_by_key(|m| m.seq);
        v.into_iter().filter(|m| !m.outgoing && !m.sender.is_empty()).map(|m| m.text).collect()
    }


    // ------------------------------------------------------------ anexos

    /// `runTransfers`: envia os anexos por enviar e descarrega os recebidos. Devolve falhas.
    pub async fn transferencias(&mut self) -> Vec<String> {
        const CHUNK: u64 = 40_960;
        let mut falhas = vec![];
        let grupos: std::collections::HashMap<String, String> =
            self.qt.groups().unwrap().into_iter().map(|g| (g.conversation_id, g.group_id)).collect();
        for r in self.qt.attachments_pending_upload().unwrap() {
            let (cid, mid) = (r.contact_id.clone(), r.message_id.clone());
            let gid = grupos.get(&cid).cloned();
            let total = self.qt.message(cid.clone(), mid.clone()).unwrap().and_then(|m| m.attachment).map(|a| a.transfer_size).unwrap_or(0);
            if self.qt.attachment_blob_id(cid.clone(), mid.clone()).unwrap().is_none() {
                let l = self.link.as_mut().unwrap();
                match l.pedido(self.qt.blob_create_request(cid.clone(), mid.clone()).unwrap()).await.unwrap() {
                    RelayResponse::BlobCreated { blob_id, token } => self.qt.attachment_set_blob(cid.clone(), mid.clone(), blob_id, token).unwrap(),
                    outro => { falhas.push(format!("criar: {}", descrever(&outro))); continue; }
                }
            }
            let mut off = 0;
            while off < total {
                let l = self.link.as_mut().unwrap();
                match l.pedido(self.qt.blob_put_request(cid.clone(), mid.clone(), off).unwrap()).await.unwrap() {
                    RelayResponse::BlobStatus { .. } => {}
                    outro => { falhas.push(format!("bloco: {}", descrever(&outro))); break; }
                }
                off += CHUNK;
            }
            if let Some(g) = gid {
                let need = self.qt.group_members_needing_session(g.clone()).unwrap();
                self.sessoes_com(need).await;
                self.qt.group_attachment_uploaded(g, mid).unwrap();
            } else {
                self.garantir_sessao(&cid).await.unwrap();
                self.qt.attachment_uploaded(cid, mid).unwrap();
            }
            self.flush().await;
        }
        for r in self.qt.attachments_pending_download().unwrap() {
            let (cid, mid) = (r.contact_id.clone(), r.message_id.clone());
            let mut have = self.qt.attachment_downloaded_bytes(cid.clone(), mid.clone()).unwrap();
            let mut total: Option<u64> = None;
            loop {
                if total.is_some_and(|t| have >= t) { break; }
                let l = self.link.as_mut().unwrap();
                match l.pedido(self.qt.blob_get_request(cid.clone(), mid.clone(), have).unwrap()).await.unwrap() {
                    RelayResponse::BlobData { size, offset, data } => {
                        total = Some(size);
                        have = self.qt.attachment_write_chunk(cid.clone(), mid.clone(), offset, data).unwrap();
                    }
                    outro => { falhas.push(format!("descarregar: {}", descrever(&outro))); break; }
                }
            }
            if total.is_some_and(|t| have >= t) {
                let done = self.qt.attachment_download_finished(cid.clone(), mid.clone()).unwrap();
                if !grupos.contains_key(&cid) && done.attachment.map(|a| a.state) == Some(qt_ffi::attachments::AttachmentState::Ready) {
                    let l = self.link.as_mut().unwrap();
                    let _ = l.pedido(self.qt.blob_delete_request(cid, mid).unwrap()).await;
                }
            }
        }
        falhas
    }

    // ------------------------------------------------------------ chave de recuperacao

    /// Liga a chave de recuperacao (palavras) e envia a copia.
    pub async fn ativar_recuperacao(&mut self) -> Vec<String> {
        let w = self.qt.recovery_enable().unwrap();
        self.copia(true).await;
        w
    }

    /// `scheduleBackup`.
    pub async fn copia(&mut self, force: bool) {
        let contacts: Vec<BackupContact> = self.contactos.iter().map(|c| BackupContact { nickname: c.nome.clone(), public_id: c.public_id.clone() }).collect();
        let up = self.qt.recovery_backup(contacts, force).unwrap();
        if up.requests.is_empty() {
            return;
        }
        let l = self.link.as_mut().expect("ligacao");
        for r in up.requests {
            match l.pedido(r).await.unwrap() {
                RelayResponse::Ack => {}
                outro => panic!("copia recusada: {}", descrever(&outro)),
            }
        }
        self.qt.recovery_backup_sent(up.digest).unwrap();
    }

    /// Desinstala a app (perde tudo) e instala de novo, sem conta.
    pub fn desinstalar(&mut self) {
        let outbox = self.outbox;
        let url = self.relays.first().map(|(_, u)| u.clone()).unwrap_or_else(|| self.url.clone());
        let mut novo = Telemovel::novo(&self.nome.clone(), &url);
        novo.outbox = outbox;
        novo.relays = self.relays.clone();
        std::mem::swap(self, &mut novo);
        drop(novo); // apaga a pasta da instalacao antiga
    }

    /// `restoreAccount` (num telemovel sem conta): copia do relay, identidade e contactos, `startLink`.
    pub async fn recuperar(&mut self, palavras: Vec<String>) -> Result<(), String> {
        self.qt.recovery_prepare(palavras).map_err(|e| format!("palavras: {e:?}"))?;
        let mut l = Ligacao::abrir_com(&self.qt, &self.url, self.pedidos.clone()).await?;
        let mut parts = vec![];
        let mut total = 1;
        while parts.len() < total {
            match l.pedido(self.qt.relay_request_backup_get(parts.len() as u32).unwrap()).await? {
                RelayResponse::BackupPart { parts: n, data, .. } => {
                    total = n as usize;
                    parts.push(data);
                }
                outro => return Err(format!("copia: {}", descrever(&outro))),
            }
        }
        drop(l);
        let acc = self.qt.recovery_restore(parts).map_err(|e| format!("restore: {e:?}"))?;
        self.contactos = acc
            .contacts
            .into_iter()
            .map(|c| Contacto { nome: c.nickname, relay_id: relay_id_of_public_id(c.public_id.clone()).unwrap(), public_id: c.public_id })
            .collect();
        self.sincronizar_contactos();
        self.ligar().await
    }
}

impl Drop for Telemovel {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Faz os ciclos de todos os telemoveis ate nada mudar (como apps abertas a receber).
pub async fn ate_acalmar(tels: &mut [&mut Telemovel]) {
    for _ in 0..12 {
        let mut n = 0;
        for t in tels.iter_mut() {
            n += t.ciclo().await;
        }
        if n == 0 {
            return;
        }
    }
}
