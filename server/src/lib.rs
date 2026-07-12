//! qt_server
//!
//! Servidor/relay: recebe mensagens ja cifradas (Double Ratchet, de
//! `core/`), guarda-as em fila, e entrega-as ao destinatario - sem nunca
//! conseguir ler o conteudo. Tambem serve como diretorio de pre-keys
//! publicas para permitir handshakes assincronos (destinatario offline).
//!
//! NOTA DE ARQUITETURA: esta implementacao vive no mesmo workspace que
//! `core/` e `transport/` por conveniencia de desenvolvimento/testes.
//! Conforme decidido em CONTRIBUTING.md, antes de um deployment real este
//! crate deve ser extraido para um repositorio separado
//! (`secure-messenger-server`), ja que quem so quer fazer self-host do
//! servidor nao precisa clonar os clientes.

pub mod connection;
pub mod protocol;
pub mod store;

pub use connection::handle_connection;
pub use store::Store;
