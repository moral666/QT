//! CLI persistente de verdade: cada comando e uma execucao SEPARADA do
//! programa, com o estado (identidade + sessoes) guardado em SQLCipher
//! entre execucoes - ao contrario de `messenger_demo.rs`, que corre tudo
//! num unico processo.
//!
//! Uso tipico (duas "pessoas", cada uma com a sua base de dados):
//!
//!   # Bob:
//!   messenger identity --db bob.sqlite --passphrase "pw-bob"
//!   messenger register --db bob.sqlite --passphrase "pw-bob" --server ws://127.0.0.1:9443
//!
//!   # Alice:
//!   messenger identity --db alice.sqlite --passphrase "pw-alice"
//!   messenger send --db alice.sqlite --passphrase "pw-alice" \
//!       --to <ID-DO-BOB-IMPRESSO-POR-'identity'> \
//!       --message "Ola Bob!" --server ws://127.0.0.1:9443
//!
//!   # Bob, mais tarde, processo completamente separado:
//!   messenger receive --db bob.sqlite --passphrase "pw-bob" --server ws://127.0.0.1:9443
//!
//! NAO usar em produção: a passphrase e passada na linha de comandos (fica
//! no historico do shell!) so por simplicidade de demo - ver
//! docs/protocol-spec.md secao 5 sobre a origem correta da passphrase
//! (Keystore/Secure Enclave). O identificador de utilizador e derivado
//! automaticamente da chave publica de identidade (nao e escolhido pelo
//! utilizador) - consistente com o objetivo de nao ter "usernames".

use clap::{Parser, Subcommand};
use secure_messenger_cli::wire_format;
use secure_messenger_core::primitives::{DhKeyPair, SigningKeyPair};
use secure_messenger_core::ratchet::{EncryptedMessage, RatchetState};
use secure_messenger_core::x3dh::{sign_pre_key, x3dh_initiate, x3dh_respond};
use secure_messenger_server::protocol::{
    deserialize_server_message, serialize_client_message, ClientMessage, ServerMessage,
};
use secure_messenger_storage::{load_identity, load_session, open_database, save_identity, save_session, StoredIdentity};
use secure_messenger_transport::{generate_static_keypair, ws_transport};

#[derive(Parser)]
#[command(name = "messenger", about = "CLI persistente do secure-messenger (demo)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Gera uma identidade nova e guarda-a na base de dados (so correr uma vez por pessoa).
    Identity {
        #[arg(long)]
        db: String,
        #[arg(long)]
        passphrase: String,
    },
    /// Publica o bundle de pre-keys no servidor, para outros poderem iniciar uma conversa.
    Register {
        #[arg(long)]
        db: String,
        #[arg(long)]
        passphrase: String,
        #[arg(long)]
        server: String,
    },
    /// Envia uma mensagem (estabelece a sessao automaticamente, se ainda nao existir).
    Send {
        #[arg(long)]
        db: String,
        #[arg(long)]
        passphrase: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        message: String,
        #[arg(long)]
        server: String,
    },
    /// Busca e decifra as mensagens em fila destinadas a esta identidade.
    Receive {
        #[arg(long)]
        db: String,
        #[arg(long)]
        passphrase: String,
        #[arg(long)]
        server: String,
    },
}

/// Identificador de utilizador derivado da chave publica de identidade
/// (nao escolhido pela pessoa) - consistente com o objetivo de anonimato.
fn derive_user_id(identity_public: &x25519_dalek::PublicKey) -> String {
    identity_public.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

async fn enviar_ao_servidor(url: &str, msg: ClientMessage) -> ServerMessage {
    let client_keys = generate_static_keypair().expect("falha ao gerar chaves Noise efemeras");
    let (mut ws_stream, mut noise) = ws_transport::client_connect(url, &client_keys.private)
        .await
        .expect("falha ao ligar/handshake com o servidor");

    let bytes = serialize_client_message(&msg);
    ws_transport::send_encrypted(&mut ws_stream, &mut noise, &bytes)
        .await
        .expect("falha ao enviar mensagem cifrada");

    let response_bytes = ws_transport::receive_encrypted(&mut ws_stream, &mut noise)
        .await
        .expect("falha ao receber resposta");
    deserialize_server_message(&response_bytes).expect("resposta do servidor malformada")
}

/// Serializa um EncryptedMessage do Double Ratchet para o formato de fio.
fn serialize_ratchet_message(msg: &EncryptedMessage) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(msg.dh_public.as_bytes());
    out.extend_from_slice(&msg.n.to_be_bytes());
    out.extend_from_slice(&msg.ciphertext);
    out
}

fn deserialize_ratchet_message(bytes: &[u8]) -> EncryptedMessage {
    let dh_public_bytes: [u8; 32] = bytes[0..32].try_into().unwrap();
    let n = u32::from_be_bytes(bytes[32..36].try_into().unwrap());
    EncryptedMessage {
        dh_public: x25519_dalek::PublicKey::from(dh_public_bytes),
        n,
        ciphertext: bytes[36..].to_vec(),
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Identity { db, passphrase } => {
            let conn = open_database(&db, &passphrase).expect("falha ao abrir a base de dados");

            let identity = StoredIdentity {
                identity: DhKeyPair::generate(),
                identity_signing: SigningKeyPair::generate(),
                signed_pre_key: DhKeyPair::generate(),
                one_time_pre_key: DhKeyPair::generate(),
            };
            let user_id = derive_user_id(&identity.identity.public);
            save_identity(&conn, &identity).expect("falha ao guardar a identidade");

            println!("Identidade criada e guardada em {db}");
            println!("O teu ID (partilha isto com quem quiser falar contigo):");
            println!("  {user_id}");
        }

        Command::Register { db, passphrase, server } => {
            let conn = open_database(&db, &passphrase).expect("falha ao abrir a base de dados");
            let identity = load_identity(&conn).expect("identidade nao encontrada - corre 'identity' primeiro");
            let user_id = derive_user_id(&identity.identity.public);

            let signature = sign_pre_key(&identity.identity_signing, &identity.signed_pre_key.public);
            let bundle_bytes = wire_format::serialize_bundle(&secure_messenger_core::x3dh::PreKeyBundle {
                identity_key: identity.identity.public,
                identity_signing_key: identity.identity_signing.verifying_key,
                signed_pre_key: identity.signed_pre_key.public,
                signed_pre_key_signature: signature,
                one_time_pre_key: Some(identity.one_time_pre_key.public),
            });

            let resp = enviar_ao_servidor(
                &server,
                ClientMessage::RegisterPreKeyBundle { user_id: user_id.clone(), bundle_bytes },
            )
            .await;
            println!("Registado como {user_id} - servidor respondeu: {resp:?}");
        }

        Command::Send { db, passphrase, to, message, server } => {
            let conn = open_database(&db, &passphrase).expect("falha ao abrir a base de dados");
            let identity = load_identity(&conn).expect("identidade nao encontrada - corre 'identity' primeiro");
            let my_user_id = derive_user_id(&identity.identity.public);

            let existing_session = load_session(&conn, &to).expect("erro ao ler sessao guardada");

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
                        ClientMessage::FetchPreKeyBundle { user_id: to.clone() },
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

            let resp = enviar_ao_servidor(
                &server,
                ClientMessage::SendMessage { from: my_user_id, to: to.clone(), ciphertext: wire_message },
            )
            .await;
            println!("Mensagem enviada para {to} - servidor respondeu: {resp:?}");

            save_session(&conn, &to, &ratchet).expect("falha ao guardar a sessao atualizada");
        }

        Command::Receive { db, passphrase, server } => {
            let conn = open_database(&db, &passphrase).expect("falha ao abrir a base de dados");
            let identity = load_identity(&conn).expect("identidade nao encontrada - corre 'identity' primeiro");
            let my_user_id = derive_user_id(&identity.identity.public);

            let resp = enviar_ao_servidor(
                &server,
                ClientMessage::FetchMessages { user_id: my_user_id },
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

            for delivered in messages {
                let bytes = delivered.ciphertext;
                let msg_type = bytes[0];

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
                    let ratchet = load_session(&conn, &delivered.from)
                        .expect("erro ao ler sessao")
                        .expect("mensagem normal recebida mas nao existe sessao com este contacto");
                    (ratchet, deserialize_ratchet_message(&bytes[1..]))
                };

                let plaintext = ratchet.decrypt(&ratchet_msg).expect("falha ao decifrar mensagem");
                println!("[{}]: {}", delivered.from, String::from_utf8_lossy(&plaintext));

                save_session(&conn, &delivered.from, &ratchet).expect("falha ao guardar sessao atualizada");
            }
        }
    }
}
