#![allow(dead_code)]
//! Utilitarios de teste: relay real em localhost que se pode MATAR e
//! REINICIAR na mesma porta (simula perda de rede / relay indisponivel).

use qt_server::{handle_connection_with, ServerConfig, Store};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};

pub struct TestRelay {
    pub url: String,
    pub port: u16,
    pub prefix: String,
    pub static_private: Vec<u8>,
    pub static_public: Vec<u8>,
    handle: Option<JoinHandle<()>>,
}

impl TestRelay {
    pub async fn start(cfg: ServerConfig) -> Self {
        let keys = qt_transport::generate_static_keypair().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let prefix = format!("t-{}-{}", std::process::id(), port);
        Self::spawn(listener, port, prefix, keys.private, keys.public, cfg)
    }

    /// Reinicia o relay na MESMA porta, com as mesmas chaves e o mesmo Redis.
    pub async fn restart(&mut self, cfg: ServerConfig) {
        self.kill();
        let mut listener = None;
        for _ in 0..50 {
            if let Ok(l) = TcpListener::bind(("127.0.0.1", self.port)).await {
                listener = Some(l);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let l = listener.expect("nao consegui reabrir a porta");
        let mut new = Self::spawn(l, self.port, self.prefix.clone(), self.static_private.clone(), self.static_public.clone(), cfg);
        self.handle = new.handle.take();
    }

    fn spawn(
        listener: TcpListener,
        port: u16,
        prefix: String,
        private: Vec<u8>,
        public: Vec<u8>,
        cfg: ServerConfig,
    ) -> Self {
        let store = Arc::new(Store::with_prefix("redis://127.0.0.1:6379", &prefix).unwrap());
        let key = private.clone();
        let handle = tokio::spawn(async move {
            // JoinSet: ao abortar esta tarefa, todas as ligacoes sao abortadas.
            let mut conns = JoinSet::new();
            loop {
                let (tcp, _) = match listener.accept().await {
                    Ok(x) => x,
                    Err(_) => break,
                };
                let (store, key, cfg) = (store.clone(), key.clone(), cfg.clone());
                conns.spawn(async move {
                    let _ = handle_connection_with(tcp, store, key, &cfg).await;
                });
            }
        });
        TestRelay {
            url: format!("ws://127.0.0.1:{port}"),
            port,
            prefix,
            static_private: private,
            static_public: public,
            handle: Some(handle),
        }
    }

    pub fn kill(&mut self) {
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        self.kill();
    }
}

pub fn fast_cfg() -> ServerConfig {
    ServerConfig {
        handshake_timeout: std::time::Duration::from_secs(5),
        auth_timeout: std::time::Duration::from_secs(5),
        idle_timeout: std::time::Duration::from_secs(60),
    }
}
