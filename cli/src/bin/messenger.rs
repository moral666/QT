//! CLI persistente de verdade: cada comando e uma execucao SEPARADA do
//! programa, com o estado (identidade + sessoes) guardado em SQLCipher
//! entre execucoes - ao contrario de `messenger_demo.rs`, que corre tudo
//! num unico processo.
//!
//! Uso tipico (duas "pessoas", cada uma com a sua base de dados). A
//! passphrase NUNCA vai na linha de comandos: e pedida no terminal,
//! mascarada com '*' (ou lida de UMA linha do stdin com --passphrase-stdin).
//!
//!   # Bob:
//!   messenger identity --db bob.sqlite
//!   messenger register --db bob.sqlite --server wss://relay.exemplo.org
//!
//!   # Alice:
//!   messenger identity --db alice.sqlite
//!   messenger send --db alice.sqlite --to <ID-DO-BOB-IMPRESSO-POR-'identity'> \
//!       --server wss://relay.exemplo.org          # a mensagem e pedida no terminal
//!
//!   # Bob, mais tarde, processo completamente separado:
//!   messenger receive --db bob.sqlite --server wss://relay.exemplo.org
//!
//!   # Scripts/CI (sem TTY):
//!   pass show qt/bob | messenger --passphrase-stdin receive --db bob.sqlite --server wss://...
//!
//! Transporte: so `wss://`. `ws://127.0.0.1` so funciona em builds de debug.
//! Ver docs/protocol-spec.md secao 5 sobre a origem da chave em apps reais
//! (Keystore/Secure Enclave). O identificador de utilizador e derivado
//! automaticamente da chave publica de identidade (nao e escolhido pelo
//! utilizador) - consistente com o objetivo de nao ter "usernames".

use clap::{Parser, Subcommand};
use qt_cli::{passphrase, wire_format};
use qt_core::primitives::{DhKeyPair, SigningKeyPair};
use qt_core::ratchet::{EncryptedMessage, RatchetState};
use qt_core::sealed_sender::{seal_sender_identity, unseal_sender_identity};
use qt_core::x3dh::{sign_pre_key, x3dh_initiate, x3dh_respond};
use qt_server::protocol::{
    ClientMessage, ServerMessage,
};
use qt_storage::{load_identity, load_session, open_database, save_identity, save_session, StoredIdentity};

#[derive(Parser)]
#[command(name = "messenger", about = "CLI persistente do secure-messenger (demo)")]
struct Cli {
    /// Le a passphrase de UMA linha do stdin em vez de a pedir no terminal
    /// (para scripts). Nunca existe uma flag com a passphrase em claro.
    #[arg(long, global = true)]
    passphrase_stdin: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Gera uma identidade nova e guarda-a na base de dados (so correr uma vez por pessoa).
    Identity {
        #[arg(long)]
        db: String,
    },
    /// Publica o bundle de pre-keys no servidor, para outros poderem iniciar uma conversa.
    Register {
        #[arg(long)]
        db: String,
        #[arg(long)]
        server: String,
    },
    /// Envia uma mensagem (estabelece a sessao automaticamente, se ainda nao existir).
    Send {
        #[arg(long)]
        db: String,
        #[arg(long)]
        to: String,
        /// Texto da mensagem. Se omitido, e pedido no terminal (assim nao fica
        /// visivel em `ps`/historico do shell).
        #[arg(long)]
        message: Option<String>,
        #[arg(long)]
        server: String,
    },
    /// Busca e decifra as mensagens em fila destinadas a esta identidade.
    Receive {
        #[arg(long)]
        db: String,
        #[arg(long)]
        server: String,
    },
}

/// Identificador de utilizador derivado da chave publica de identidade
/// (nao escolhido pela pessoa) - consistente com o objetivo de anonimato.
fn derive_user_id(identity_public: &x25519_dalek::PublicKey) -> String {
    identity_public.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// O inverso de `derive_user_id`: o ID de um contacto E a sua chave publica
/// de identidade, em hex - por isso conseguimos "endereca-lo" para o
/// sealed sender sem precisar de o ter contactado antes.
fn hex_to_pubkey(hex_str: &str) -> x25519_dalek::PublicKey {
    let bytes: Vec<u8> = (0..hex_str.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).expect("ID invalido: nao e hex valido"))
        .collect();
    let arr: [u8; 32] = bytes.try_into().expect("ID invalido: deveria ter 32 bytes (64 chars hex)");
    x25519_dalek::PublicKey::from(arr)
}

async fn enviar_ao_servidor(url: &str, signing: [u8; 32], msg: ClientMessage) -> ServerMessage {
    match qt_server::client::one_shot(url, signing, msg).await {
        Ok(r) => r,
        Err(qt_server::client::RelayError::InsecureUrl) => {
            fatal("o servidor tem de usar wss:// (TLS); ws:// so e aceite em builds de debug para localhost")
        }
        Err(e) => fatal(&format!("falha ao falar com o relay: {e}")),
    }
}

/// ID publico da CLI = "<identidade_hex>:<assinatura_hex>". A caixa de
/// correio no relay e `<assinatura_hex>` (autenticada por assinatura Ed25519);
/// `<identidade_hex>` serve para o sealed sender e para as sessoes.
fn cli_public_id(identity: &StoredIdentity) -> String {
    format!(
        "{}:{}",
        derive_user_id(&identity.identity.public),
        qt_server::protocol::user_id_from_signing_public(&identity.identity_signing.verifying_key.to_bytes())
    )
}

fn mailbox_of(identity: &StoredIdentity) -> String {
    qt_server::protocol::user_id_from_signing_public(&identity.identity_signing.verifying_key.to_bytes())
}

fn split_public_id(id: &str) -> (String, String) {
    let (a, b) = id.split_once(':').expect("ID invalido: esperado <identidade_hex>:<assinatura_hex>");
    (a.to_string(), b.to_string())
}

/// Serializa um EncryptedMessage do Double Ratchet para o formato de fio.
fn serialize_ratchet_message(msg: &EncryptedMessage) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(msg.dh_public.as_bytes());
    out.extend_from_slice(&msg.pn.to_be_bytes());
    out.extend_from_slice(&msg.n.to_be_bytes());
    out.extend_from_slice(&msg.ciphertext);
    out
}

fn deserialize_ratchet_message(bytes: &[u8]) -> EncryptedMessage {
    let dh_public_bytes: [u8; 32] = bytes[0..32].try_into().unwrap();
    let pn = u32::from_be_bytes(bytes[32..36].try_into().unwrap());
    let n = u32::from_be_bytes(bytes[36..40].try_into().unwrap());
    EncryptedMessage {
        dh_public: x25519_dalek::PublicKey::from(dh_public_bytes),
        pn,
        n,
        ciphertext: bytes[40..].to_vec(),
    }
}

/// Erro fatal: mensagem curta e saida != 0 (sem panic/backtrace que possa
/// incluir dados sensiveis).
fn fatal(msg: &str) -> ! {
    eprintln!("erro: {msg}");
    std::process::exit(1);
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Identity cria a base de dados -> confirmacao + comprimento minimo.
    let mode = match cli.command {
        Command::Identity { .. } => passphrase::Mode::Create,
        _ => passphrase::Mode::Unlock,
    };
    let passphrase = passphrase::obtain(mode, cli.passphrase_stdin).unwrap_or_else(|e| fatal(&e.to_string()));

    match cli.command {
        Command::Identity { db } => {
            let conn = open_database(&db, passphrase.as_str()).expect("falha ao abrir a base de dados");

            let identity = StoredIdentity {
                identity: DhKeyPair::generate(),
                identity_signing: SigningKeyPair::generate(),
                signed_pre_key: DhKeyPair::generate(),
                one_time_pre_key: DhKeyPair::generate(),
            };
            let user_id = cli_public_id(&identity);
            save_identity(&conn, &identity).expect("falha ao guardar a identidade");

            println!("Identidade criada e guardada em {db}");
            println!("O teu ID (partilha isto com quem quiser falar contigo):");
            println!("  {user_id}");
        }

        Command::Register { db, server } => {
            let conn = open_database(&db, passphrase.as_str()).expect("falha ao abrir a base de dados");
            let identity = load_identity(&conn).expect("identidade nao encontrada - corre 'identity' primeiro");
            let user_id = mailbox_of(&identity);

            let signature = sign_pre_key(&identity.identity_signing, &identity.signed_pre_key.public);
            let bundle_bytes = wire_format::serialize_bundle(&qt_core::x3dh::PreKeyBundle {
                identity_key: identity.identity.public,
                identity_signing_key: identity.identity_signing.verifying_key,
                signed_pre_key: identity.signed_pre_key.public,
                signed_pre_key_signature: signature,
                one_time_pre_key: Some(identity.one_time_pre_key.public),
            });

            let resp = enviar_ao_servidor(
                &server,
                identity.identity_signing.to_bytes(),
                ClientMessage::RegisterPreKeyBundle { user_id: user_id.clone(), bundle_bytes },
            )
            .await;
            println!("Registado como {user_id} - servidor respondeu: {resp:?}");
        }

        Command::Send { db, to, message, server } => {
            let message = match message {
                Some(m) => m,
                None => inquire::Text::new("Mensagem:").prompt().unwrap_or_else(|_| fatal("mensagem nao fornecida")),
            };
            let conn = open_database(&db, passphrase.as_str()).expect("falha ao abrir a base de dados");
            let identity = load_identity(&conn).expect("identidade nao encontrada - corre 'identity' primeiro");

            let (to_identity_hex, to_mailbox) = split_public_id(&to);
            let existing_session = load_session(&conn, &to_identity_hex).expect("erro ao ler sessao guardada");

            let (ratchet, wire_message) = match existing_session {
                Some(mut ratchet) => {
                    // Sessao ja existe - mensagem normal, sem cabecalho X3DH extra.
                    let encrypted = ratchet.encrypt(message.as_bytes()).expect("falha ao cifrar");
                    let mut wire = vec![0u8]; // type = 0 (mensagem normal)
                    wire.extend_from_slice(&serialize_ratchet_message(&encrypted));
                    (ratchet, wire)
                }
                None => {
                    // Primeira mensagem para este contacto - busca o bundle dele
                    // e faz o handshake X3DH.
                    let resp = enviar_ao_servidor(
                        &server,
                        identity.identity_signing.to_bytes(),
                        ClientMessage::FetchPreKeyBundle { user_id: to_mailbox.clone() },
                    )
                    .await;
                    let their_bundle_bytes = match resp {
                        ServerMessage::PreKeyBundle { bundle_bytes } => bundle_bytes,
                        ServerMessage::PreKeyBundleNotFound => {
                            panic!("'{to}' ainda nao publicou um bundle - pede-lhe para correr 'register' primeiro")
                        }
                        other => panic!("resposta inesperada: {other:?}"),
                    };
                    let their_bundle = wire_format::deserialize_bundle(&their_bundle_bytes)
                        .expect("bundle recebido esta corrompido");

                    let init_result = x3dh_initiate(&identity.identity, &their_bundle)
                        .expect("assinatura da signed pre-key invalida - handshake abortado");

                    let mut ratchet = RatchetState::init_as_initiator(
                        init_result.shared_secret,
                        their_bundle.signed_pre_key,
                    );
                    let encrypted = ratchet.encrypt(message.as_bytes()).expect("falha ao cifrar");

                    // type = 1 (mensagem inicial): inclui a nossa identity_key
                    // publica + a ephemeral key do X3DH, para o destinatario
                    // conseguir completar o handshake do lado dele.
                    let mut wire = vec![1u8];
                    wire.extend_from_slice(identity.identity.public.as_bytes());
                    wire.extend_from_slice(init_result.ephemeral_public.as_bytes());
                    wire.extend_from_slice(&serialize_ratchet_message(&encrypted));
                    (ratchet, wire)
                }
            };

            let their_identity_pub = hex_to_pubkey(&to_identity_hex);
            let sealed_from = seal_sender_identity(&identity.identity.public, &their_identity_pub).expect("falha ao selar o remetente");

            let resp = enviar_ao_servidor(
                &server,
                identity.identity_signing.to_bytes(),
                ClientMessage::SendMessage { to: to_mailbox.clone(), sealed_from, ciphertext: wire_message },
            )
            .await;
            println!("Mensagem enviada para {to} - servidor respondeu: {resp:?}");

            save_session(&conn, &to_identity_hex, &ratchet).expect("falha ao guardar a sessao atualizada");
        }

        Command::Receive { db, server } => {
            let conn = open_database(&db, passphrase.as_str()).expect("falha ao abrir a base de dados");
            let identity = load_identity(&conn).expect("identidade nao encontrada - corre 'identity' primeiro");
            let my_user_id = mailbox_of(&identity);

            let resp = enviar_ao_servidor(
                &server,
                identity.identity_signing.to_bytes(),
                ClientMessage::FetchMessages { user_id: my_user_id.clone() },
            )
            .await;
            let messages = match resp {
                ServerMessage::Messages { messages } => messages,
                other => panic!("resposta inesperada: {other:?}"),
            };

            if messages.is_empty() {
                println!("Sem mensagens novas.");
                return;
            }

            let mut acked: Vec<u64> = Vec::new();
            for delivered in messages {
                acked.push(delivered.id);
                let bytes = delivered.ciphertext;
                let msg_type = bytes[0];

                // Abre o envelope selado PRIMEIRO - so agora sabemos quem
                // enviou (o servidor nunca soube isto).
                let sender_identity_pub = unseal_sender_identity(&identity.identity, &delivered.sealed_from)
                    .expect("falha ao abrir o envelope selado - remetente desconhecido ou corrompido");
                let sender_user_id = derive_user_id(&sender_identity_pub);

                let (mut ratchet, ratchet_msg) = if msg_type == 1 {
                    // Primeira mensagem deste contacto - completar o X3DH do
                    // nosso lado (responder).
                    let sender_identity_bytes: [u8; 32] = bytes[1..33].try_into().unwrap();
                    let ephemeral_bytes: [u8; 32] = bytes[33..65].try_into().unwrap();
                    let sender_identity = x25519_dalek::PublicKey::from(sender_identity_bytes);
                    let ephemeral = x25519_dalek::PublicKey::from(ephemeral_bytes);

                    let shared_secret = x3dh_respond(
                        &identity.identity,
                        &identity.signed_pre_key,
                        Some(&identity.one_time_pre_key),
                        &sender_identity,
                        &ephemeral,
                    );
                    let signed_pre_key_clone = DhKeyPair {
                        private: x25519_dalek::StaticSecret::from(identity.signed_pre_key.private.to_bytes()),
                        public: identity.signed_pre_key.public,
                    };
                    let ratchet = RatchetState::init_as_responder(shared_secret, signed_pre_key_clone);
                    (ratchet, deserialize_ratchet_message(&bytes[65..]))
                } else {
                    let ratchet = load_session(&conn, &sender_user_id)
                        .expect("erro ao ler sessao")
                        .expect("mensagem normal recebida mas nao existe sessao com este contacto");
                    (ratchet, deserialize_ratchet_message(&bytes[1..]))
                };

                let plaintext = ratchet.decrypt(&ratchet_msg).expect("falha ao decifrar mensagem");
                println!("[{sender_user_id}]: {}", String::from_utf8_lossy(&plaintext));

                save_session(&conn, &sender_user_id, &ratchet).expect("falha ao guardar sessao atualizada");
            }
            // So agora (tudo processado e persistido) o relay pode apagar as mensagens.
            let _ = enviar_ao_servidor(
                &server,
                identity.identity_signing.to_bytes(),
                ClientMessage::AckMessages { user_id: my_user_id, ids: acked },
            )
            .await;
        }
    }
}
