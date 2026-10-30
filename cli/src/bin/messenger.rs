//! `messenger`: o SEALER no terminal.
//!
//! Usa EXATAMENTE o mesmo nucleo que as apps Android e iPhone (`QtClient`:
//! PQXDH + Double Ratchet + Sealed Sender, historico em SQLCipher, recibos)
//! e fala com o mesmo relay, por isso troca mensagens com os telemoveis.
//! Cada comando e uma execucao separada; o estado fica na base de dados
//! cifrada entre execucoes.
//!
//!   messenger identity --db eu.sqlite                    # cria a identidade (uma vez)
//!   messenger id       --db eu.sqlite                    # mostra o teu QT Public ID
//!   messenger add      --db eu.sqlite Ana qt1:...        # guarda um contacto
//!   messenger send     --db eu.sqlite --to Ana --server wss://... --pin <hex>
//!   messenger receive  --db eu.sqlite --server wss://... --pin <hex>
//!   messenger listen   --db eu.sqlite --server wss://... --pin <hex>   # fica a receber
//!   messenger history  --db eu.sqlite Ana
//!   messenger contacts --db eu.sqlite
//!   messenger remove   --db eu.sqlite Ana                # apaga contacto, mensagens e chaves
//!
//! O relay tambem pode vir das variaveis QT_RELAY_URL e QT_RELAY_PIN.
//! A passphrase NUNCA vai na linha de comandos: e pedida no terminal,
//! mascarada com '*' (ou lida de UMA linha do stdin com --passphrase-stdin).
//! So `wss://` (TLS); `ws://127.0.0.1` so em builds de debug.

use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use qt_cli::passphrase::{self, Mode};
use qt_cli::store::{self, Contact, Keys};
use qt_ffi::history::{MessageStatus, ReceiveResult};
use qt_ffi::*;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

#[derive(Parser)]
#[command(name = "messenger", about = "SEALER no terminal (mesmo nucleo e relay que as apps)")]
struct Cli {
    /// Le a passphrase de UMA linha do stdin em vez de a pedir no terminal
    /// (para scripts). Nunca existe uma flag com a passphrase em claro.
    #[arg(long, global = true)]
    passphrase_stdin: bool,

    /// Base de dados cifrada desta identidade.
    #[arg(long, global = true, default_value = "messenger.sqlite")]
    db: String,

    /// URL do relay (wss://...). Tambem: variavel QT_RELAY_URL.
    #[arg(long, global = true, env = "QT_RELAY_URL")]
    server: Option<String>,

    /// Chave publica Noise do relay (64 hex), para pinning. Tambem: QT_RELAY_PIN.
    #[arg(long, global = true, env = "QT_RELAY_PIN")]
    pin: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Cria a base de dados e a identidade (so uma vez por pessoa).
    Identity,
    /// Mostra o teu QT Public ID (o que partilhas com os contactos).
    Id,
    /// Publica as tuas chaves publicas no relay (o receive e o listen tambem o fazem).
    Register,
    /// Guarda um contacto: um nome e o QT Public ID dele (qt1:...).
    Add { name: String, public_id: String },
    /// Lista os contactos.
    Contacts,
    /// Apaga um contacto PARA SEMPRE: mensagens, envios pendentes e chaves da conversa.
    Remove { name: String },
    /// Envia uma mensagem. Se --message faltar, o texto e pedido no terminal.
    Send {
        #[arg(long)]
        to: String,
        #[arg(long)]
        message: Option<String>,
    },
    /// Vai buscar as mensagens novas ao relay e mostra-as.
    Receive,
    /// Fica a receber mensagens ate carregares Ctrl+C.
    Listen {
        /// Segundos entre verificacoes.
        #[arg(long, default_value_t = 3)]
        every: u64,
    },
    /// Mostra o historico de uma conversa (guardado na base de dados cifrada).
    History {
        name: String,
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
}

fn fatal(msg: &str) -> ! {
    eprintln!("erro: {msg}");
    std::process::exit(1)
}

fn s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// ============================================================== base de dados

struct Local {
    qt: Arc<QtClient>,
    keys: Keys,
    db: String,
    contacts: Vec<Contact>,
}

impl Local {
    fn open(cli: &Cli) -> Local {
        let salt = store::load_salt(&cli.db).unwrap_or_else(|e| fatal(&e));
        let pass = passphrase::obtain(Mode::Unlock, cli.passphrase_stdin).unwrap_or_else(|e| fatal(&e.to_string()));
        let keys = store::derive_keys(&pass, &salt).unwrap_or_else(|e| fatal(&e));
        drop(pass);
        let qt = QtClient::open(cli.db.clone(), keys.db.to_vec())
            .unwrap_or_else(|_| fatal("passphrase errada ou base de dados danificada"));
        if !qt.has_identity().unwrap_or(false) {
            fatal("esta base de dados nao tem identidade: corre `messenger identity`");
        }
        let contacts = store::load_contacts(&cli.db, &keys).unwrap_or_else(|e| fatal(&e));
        Local { qt, keys, db: cli.db.clone(), contacts }
    }

    fn save_contacts(&self) {
        store::save_contacts(&self.db, &self.keys, &self.contacts).unwrap_or_else(|e| fatal(&e));
    }

    fn find(&self, name_or_id: &str) -> Option<&Contact> {
        self.contacts
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name_or_id) || c.public_id == name_or_id)
    }

    fn name_of(&self, contact_id: &str) -> String {
        match self.contacts.iter().find(|c| c.contact_id() == contact_id) {
            Some(c) => c.name.clone(),
            None => format!("desconhecido ({}…)", &contact_id[..contact_id.len().min(8)]),
        }
    }
}

// ===================================================================== relay

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Ligacao ao relay: WebSocket + Noise (NoiseSession do nucleo), com pinning
/// e autenticacao, exatamente como o RelayLink das apps.
struct Relay {
    ws: Ws,
    noise: Arc<NoiseSession>,
}

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

impl Relay {
    async fn connect(qt: &Arc<QtClient>, url: &str, pin: &Option<String>) -> Result<Relay, String> {
        qt_transport::ws_transport::check_url_policy(url)
            .map_err(|_| "o relay tem de usar wss:// (ws:// so em builds de debug para localhost)".to_string())?;
        let noise = qt.relay_noise_session().map_err(s)?;
        let (mut ws, _) = tokio::time::timeout(Duration::from_secs(10), tokio_tungstenite::connect_async(url))
            .await
            .map_err(|_| "tempo esgotado a ligar ao relay".to_string())?
            .map_err(|e| format!("nao foi possivel ligar ao relay: {e}"))?;
        while !noise.is_finished() {
            ws.send(Message::Binary(noise.write_step().map_err(s)?)).await.map_err(s)?;
            if !noise.is_finished() {
                noise.read_step(next_binary(&mut ws).await?).map_err(s)?;
            }
        }
        if let Some(p) = pin {
            let got: String = noise.remote_static_public_key().map_err(s)?.iter().map(|b| format!("{b:02x}")).collect();
            if !got.eq_ignore_ascii_case(p.trim()) {
                return Err("a chave do relay e diferente da esperada (--pin): possivel intermediario, ligacao recusada".into());
            }
        }
        let mut r = Relay { ws, noise };
        let auth = qt.relay_auth_message(r.noise.handshake_hash().map_err(s)?).map_err(s)?;
        match r.request(auth).await? {
            RelayResponse::Ack => Ok(r),
            _ => Err("o relay recusou a autenticacao".into()),
        }
    }

    async fn request(&mut self, req: Vec<u8>) -> Result<RelayResponse, String> {
        self.ws.send(Message::Binary(self.noise.encrypt(req).map_err(s)?)).await.map_err(s)?;
        let frame = next_binary(&mut self.ws).await?;
        relay_parse_response(self.noise.decrypt(frame).map_err(s)?).map_err(s)
    }

    async fn close(mut self) {
        let _ = self.ws.close(None).await;
    }
}

async fn next_binary(ws: &mut Ws) -> Result<Vec<u8>, String> {
    loop {
        match tokio::time::timeout(REQUEST_TIMEOUT, ws.next()).await {
            Err(_) => return Err("tempo esgotado a espera do relay".into()),
            Ok(None) => return Err("o relay fechou a ligacao".into()),
            Ok(Some(Err(e))) => return Err(s(e)),
            Ok(Some(Ok(Message::Binary(b)))) => return Ok(b),
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
            Ok(Some(Ok(_))) => return Err("resposta inesperada do relay".into()),
        }
    }
}

fn describe(r: &RelayResponse) -> String {
    match r {
        RelayResponse::Ack => "Ack".into(),
        RelayResponse::Bundle { .. } => "Bundle".into(),
        RelayResponse::BundleNotFound => "BundleNotFound".into(),
        RelayResponse::Messages { .. } => "Messages".into(),
        RelayResponse::Error { reason } => reason.clone(),
        // Anexos: a CLI nao os envia nem recebe.
        RelayResponse::BlobCreated { .. }
        | RelayResponse::BlobStatus { .. }
        | RelayResponse::BlobData { .. }
        | RelayResponse::BlobNotFound => "Anexo".into(),
        RelayResponse::BackupPart { .. } | RelayResponse::BackupNotFound => "Copia de recuperacao".into(),
        RelayResponse::Premium { .. } => "Premium".into(),
        // Fichas do video: a CLI nao faz chamadas.
        RelayResponse::Fichas { .. } => "Fichas".into(),
        // Comunidade (0.9.52): so as apps a mostram.
        RelayResponse::CommunityVersion { .. }
        | RelayResponse::CommunityIndex { .. }
        | RelayResponse::CommunityNone
        | RelayResponse::CommunityPhotos { .. } => "Comunidade".into(),
    }
}

async fn connect(local: &Local, cli: &Cli) -> Relay {
    let url = cli
        .server
        .clone()
        .unwrap_or_else(|| fatal("falta o relay: usa --server wss://... (ou a variavel QT_RELAY_URL)"));
    if cli.pin.is_none() {
        eprintln!("aviso: sem --pin, a identidade do relay nao e verificada (recomendado em producao)");
    }
    Relay::connect(&local.qt, &url, &cli.pin).await.unwrap_or_else(|e| fatal(&e))
}

async fn register(local: &Local, relay: &mut Relay) {
    let req = local
        .qt
        .relay_request_register_bundle(local.qt.publishable_bundle().unwrap_or_else(|e| fatal(&s(e))))
        .unwrap_or_else(|e| fatal(&s(e)));
    match relay.request(req).await {
        Ok(RelayResponse::Ack) => {}
        Ok(other) => fatal(&format!("o relay recusou as chaves publicas: {}", describe(&other))),
        Err(e) => fatal(&e),
    }
}

/// Envia ao relay tudo o que esta na outbox (mensagens e recibos), por ordem.
/// O que o relay nao confirmar fica guardado e volta a ser tentado depois.
async fn flush_outbox(local: &Local, relay: &mut Relay) -> (usize, usize) {
    let Ok(items) = local.qt.outbox() else { return (0, 0) };
    let (mut ok, mut left) = (0, 0);
    for it in items {
        let Some(c) = local.contacts.iter().find(|c| c.contact_id() == it.contact_id) else {
            left += 1; // contacto desconhecido: fica a espera de ser adicionado
            continue;
        };
        let Ok(relay_id) = relay_id_of_public_id(c.public_id.clone()) else { continue };
        let Ok(req) = local.qt.relay_request_send(relay_id, it.blob) else { continue };
        match relay.request(req).await {
            Ok(RelayResponse::Ack) => {
                let _ = local.qt.outbox_sent(it.outbox_id);
                ok += 1;
            }
            Ok(_) => left += 1,
            Err(_) => {
                left += 1;
                break; // sem rede: mantem a ordem e tenta mais tarde
            }
        }
    }
    (ok, left)
}

/// Busca, decifra e guarda as mensagens novas; so confirma ao relay DEPOIS de
/// guardar. Devolve quantas mensagens novas foram mostradas.
async fn receive_once(local: &Local, relay: &mut Relay, quiet_when_empty: bool) -> Result<usize, String> {
    let mut shown = 0;
    let mut touched: Vec<String> = Vec::new();
    for _ in 0..20 {
        let req = local.qt.relay_request_fetch().map_err(s)?;
        let msgs = match relay.request(req).await? {
            RelayResponse::Messages { messages } => messages,
            other => return Err(format!("resposta inesperada do relay: {}", describe(&other))),
        };
        if msgs.is_empty() {
            break;
        }
        let mut ack = Vec::new();
        for m in msgs {
            #[allow(unreachable_patterns)] // versoes futuras do nucleo podem acrescentar casos
            match local.qt.receive_sealed(m.ciphertext) {
                Ok(ReceiveResult::NewMessage { message }) => {
                    println!("[{}] {}: {}", hhmm(message.local_at_ms), local.name_of(&message.contact_id), message.text);
                    if !touched.contains(&message.contact_id) {
                        touched.push(message.contact_id.clone());
                    }
                    shown += 1;
                    ack.push(m.id);
                }
                Ok(ReceiveResult::StatusChanged { contact_id, message_ids, status }) => {
                    let what = match status {
                        MessageStatus::Read => "vista(s)",
                        _ => "entregue(s)",
                    };
                    println!("        {} mensagem(ns) para {} {}", message_ids.len(), local.name_of(&contact_id), what);
                    ack.push(m.id);
                }
                Ok(_) => ack.push(m.id),
                // Problema local: NAO confirmar, o relay volta a entregar mais tarde.
                Err(QtError::Storage) => {}
                // Repetida, corrompida ou de uma conversa que ja nao existe: nada a recuperar.
                Err(_) => {
                    eprintln!("aviso: uma mensagem nao pode ser lida e foi descartada");
                    ack.push(m.id);
                }
            }
        }
        if !ack.is_empty() {
            let req = local.qt.relay_request_ack(ack).map_err(s)?;
            relay.request(req).await?;
        }
    }
    // Recibos: "entregue" para tudo o que chegou e "vista" para o que foi mostrado.
    let _ = local.qt.prepare_delivery_receipts();
    for cid in &touched {
        let _ = local.qt.mark_conversation_read(cid.clone(), true);
    }
    flush_outbox(local, relay).await;
    if shown == 0 && !quiet_when_empty {
        println!("(sem mensagens novas)");
    }
    Ok(shown)
}

fn hhmm(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let day = secs.rem_euclid(86_400);
    format!("{:02}:{:02} UTC", day / 3600, (day % 3600) / 60)
}

fn status_glyph(s: &MessageStatus) -> &'static str {
    match s {
        MessageStatus::Pending => "por enviar",
        MessageStatus::Sent => "enviada",
        MessageStatus::Delivered => "entregue",
        MessageStatus::Read => "vista",
        MessageStatus::Received | MessageStatus::ReadByMe => "",
    }
}

// ===================================================================== main

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match &cli.command {
        Command::Identity => {
            let salt = store::create_salt(&cli.db).unwrap_or_else(|e| fatal(&e));
            let pass = match passphrase::obtain(Mode::Create, cli.passphrase_stdin) {
                Ok(p) => p,
                Err(e) => {
                    let _ = std::fs::remove_file(store::salt_path(&cli.db));
                    fatal(&e.to_string())
                }
            };
            let keys = store::derive_keys(&pass, &salt).unwrap_or_else(|e| fatal(&e));
            drop(pass);
            let qt = QtClient::open(cli.db.clone(), keys.db.to_vec()).unwrap_or_else(|e| fatal(&s(e)));
            store::restrict_permissions(&cli.db);
            let me = qt.create_identity().unwrap_or_else(|e| fatal(&s(e)));
            println!("Identidade criada em {}.\n\nO teu QT Public ID (partilha-o com quem quiseres falar):\n{}", cli.db, me.public_id);
            println!("\nGuarda bem a passphrase: sem ela, esta base de dados nao abre e nao ha recuperacao.");
        }
        Command::Id => {
            let local = Local::open(&cli);
            println!("{}", local.qt.export_public_identity().unwrap_or_else(|e| fatal(&s(e))).public_id);
        }
        Command::Register => {
            let local = Local::open(&cli);
            let mut relay = connect(&local, &cli).await;
            register(&local, &mut relay).await;
            relay.close().await;
            println!("Chaves publicas publicadas no relay.");
        }
        Command::Add { name, public_id } => {
            let mut local = Local::open(&cli);
            let name = name.trim().to_string();
            let public_id = public_id.trim().to_string();
            if !store::valid_name(&name) {
                fatal("nome invalido (1 a 40 caracteres, sem tabs nem mudancas de linha)");
            }
            if relay_id_of_public_id(public_id.clone()).is_err() {
                fatal("QT Public ID invalido (esperado qt1: seguido de 128 caracteres hex)");
            }
            let me = local.qt.export_public_identity().unwrap_or_else(|e| fatal(&s(e)));
            if me.public_id == public_id {
                fatal("esse e o teu proprio ID");
            }
            if local.contacts.iter().any(|c| c.name.eq_ignore_ascii_case(&name) && c.public_id != public_id) {
                fatal("ja tens um contacto com esse nome");
            }
            local.contacts.retain(|c| c.public_id != public_id);
            local.contacts.push(Contact { name: name.clone(), public_id });
            local.save_contacts();
            println!("Contacto \"{name}\" guardado.");
        }
        Command::Contacts => {
            let local = Local::open(&cli);
            if local.contacts.is_empty() {
                println!("(ainda nao tens contactos: usa `messenger add <nome> <qt1:...>`)");
            }
            for c in &local.contacts {
                println!("{:<20} {}…", c.name, &c.public_id[..c.public_id.len().min(16)]);
            }
        }
        Command::Remove { name } => {
            let mut local = Local::open(&cli);
            let Some(c) = local.find(name).cloned() else { fatal("nao existe esse contacto") };
            local.qt.delete_contact(c.contact_id()).unwrap_or_else(|e| fatal(&s(e)));
            local.contacts.retain(|x| x.public_id != c.public_id);
            local.save_contacts();
            println!("Contacto \"{}\" apagado, com as mensagens e as chaves da conversa.", c.name);
        }
        Command::Send { to, message } => {
            let local = Local::open(&cli);
            let Some(c) = local.find(to).cloned() else {
                fatal("contacto desconhecido: adiciona-o primeiro com `messenger add <nome> <qt1:...>`")
            };
            let text = match message {
                Some(t) => t.clone(),
                None if cli.passphrase_stdin => fatal("com --passphrase-stdin usa --message"),
                None => inquire::Text::new(&format!("Mensagem para {}:", c.name))
                    .prompt()
                    .unwrap_or_else(|_| fatal("cancelado")),
            };
            if text.trim().is_empty() {
                fatal("mensagem vazia");
            }
            let mut relay = connect(&local, &cli).await;
            let cid = c.contact_id();
            if !local.qt.has_session(cid.clone()).unwrap_or(false) {
                let relay_id = relay_id_of_public_id(c.public_id.clone()).unwrap_or_else(|e| fatal(&s(e)));
                let req = local.qt.relay_request_fetch_bundle(relay_id).unwrap_or_else(|e| fatal(&s(e)));
                match relay.request(req).await {
                    Ok(RelayResponse::Bundle { bytes }) => {
                        let bundle = peer_bundle_from_relay_bytes(bytes)
                            .unwrap_or_else(|_| fatal("o relay devolveu chaves invalidas para este contacto"));
                        local
                            .qt
                            .start_session(c.public_id.clone(), bundle)
                            .unwrap_or_else(|_| fatal("as chaves do relay nao correspondem ao ID deste contacto"));
                    }
                    Ok(RelayResponse::BundleNotFound) => {
                        fatal("o contacto ainda nao publicou as chaves (tem de abrir a app uma vez)")
                    }
                    Ok(other) => fatal(&format!("resposta inesperada do relay: {}", describe(&other))),
                    Err(e) => fatal(&e),
                }
            }
            // Numa so transacao no nucleo: ratchet avanca + historico + outbox.
            let item = local.qt.send_text(cid.clone(), text.trim().to_string()).unwrap_or_else(|e| fatal(&s(e)));
            flush_outbox(&local, &mut relay).await;
            relay.close().await;
            let st = local
                .qt
                .message(cid, item.outbox_id)
                .ok()
                .flatten()
                .map(|m| status_glyph(&m.status))
                .unwrap_or("por enviar");
            println!("Mensagem para {}: {st}.", c.name);
        }
        Command::Receive => {
            let local = Local::open(&cli);
            let mut relay = connect(&local, &cli).await;
            register(&local, &mut relay).await;
            if let Err(e) = receive_once(&local, &mut relay, false).await {
                fatal(&e);
            }
            relay.close().await;
        }
        Command::Listen { every } => {
            let local = Local::open(&cli);
            let url = cli.server.clone().unwrap_or_else(|| fatal("falta o relay: usa --server wss://..."));
            let mut relay = connect(&local, &cli).await;
            register(&local, &mut relay).await;
            println!("A receber mensagens (Ctrl+C para sair)...");
            let mut link = Some(relay);
            loop {
                // Ligacao perdida (rede, relay reiniciado): volta a ligar com uma sessao Noise nova.
                if link.is_none() {
                    match Relay::connect(&local.qt, &url, &cli.pin).await {
                        Ok(r) => {
                            eprintln!("(ligacao ao relay restabelecida)");
                            link = Some(r);
                        }
                        Err(e) if e.contains("intermediario") => fatal(&e),
                        Err(_) => {}
                    }
                }
                if let Some(r) = link.as_mut() {
                    if let Err(e) = receive_once(&local, r, true).await {
                        eprintln!("(sem ligacao ao relay: {e}; a tentar de novo)");
                        link = None;
                    }
                }
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => break,
                    _ = tokio::time::sleep(Duration::from_secs((*every).max(1))) => {}
                }
            }
            if let Some(r) = link {
                r.close().await;
            }
        }
        Command::History { name, limit } => {
            let local = Local::open(&cli);
            let Some(c) = local.find(name).cloned() else { fatal("nao existe esse contacto") };
            let mut msgs = local
                .qt
                .messages_page(c.contact_id(), None, (*limit).clamp(1, 500))
                .unwrap_or_else(|e| fatal(&s(e)));
            msgs.reverse();
            if msgs.is_empty() {
                println!("(sem mensagens com {})", c.name);
            }
            for m in msgs {
                if m.outgoing {
                    println!("[{}] eu: {}  ({})", hhmm(m.local_at_ms), m.text, status_glyph(&m.status));
                } else {
                    println!("[{}] {}: {}", hhmm(m.local_at_ms), c.name, m.text);
                }
            }
        }
    }
}
