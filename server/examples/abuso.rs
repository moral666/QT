//! TESTE DE ABUSO CONTROLADO do relay: tenta encher o Redis como faria um
//! atacante e verifica se um par de contas normais continua a conversar.
//!
//! Local (relay e Redis de teste, ver docs/performance.md, secção "Abuso"):
//!   cargo run -p qt_server --example abuso -- --relay ws://127.0.0.1:9550 \
//!       --modo descartaveis --ligacoes 20 --msgs 300
//!
//! Produção (pequeno, depois de atualizar o relay; só confirma que é recusado):
//!   cargo run --release -p qt_server --example abuso -- --relay wss://relay.sealer.pt \
//!       --pin <pin> --modo descartaveis --ligacoes 1 --msgs 3
//!
//! Modos:
//! - `descartaveis`: cada ligação usa uma chave nova, sem registo nem prova de
//!   trabalho, e envia `--msgs` mensagens de `--tamanho` bytes para um ID que nunca
//!   existiu. Tenta também registar um token de notificações.
//! - `contas`: regista `--contas` contas (com prova de trabalho, cronometrada) e cada
//!   uma envia `--msgs` mensagens para a seguinte. No fim apaga as contas (e as filas).
//!
//! Antes e depois do ataque, duas contas registadas trocam uma mensagem: escreve
//! "SERVICO NORMAL: OK" ou "FALHOU". Num relay que não seja local, o volume fica
//! limitado a 1 ligação e 10 mensagens (é um teste controlado, não um ataque).

use qt_core::primitives::{DhKeyPair, SigningKeyPair};
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::protocol::*;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn arg(name: &str, default: &str) -> String {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned()).unwrap_or_else(|| default.to_string())
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).filter_map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()).collect()
}

fn cfg(pin: &Option<Vec<u8>>) -> RelayConfig {
    RelayConfig {
        connect_timeout: Duration::from_secs(10),
        request_timeout: Duration::from_secs(20),
        max_retries: 0,
        backoff_base: Duration::from_millis(100),
        expected_server_static: pin.clone(),
    }
}

async fn ligar(url: &str, pin: &Option<Vec<u8>>, s: &SigningKeyPair) -> Result<RelayClient, String> {
    let n = qt_transport::generate_static_keypair().map_err(|e| e.to_string())?;
    RelayClient::connect(url, n.private, s.to_bytes(), cfg(pin)).await.map_err(|e| format!("{e:?}"))
}

fn uid(s: &SigningKeyPair) -> String {
    user_id_from_signing_public(&s.verifying_key.to_bytes())
}

/// Bundle v3 válido (o relay só verifica a assinatura e o dono).
fn bundle(s: &SigningKeyPair) -> Vec<u8> {
    let id = DhKeyPair::generate();
    let spk = DhKeyPair::generate();
    qt_core::identity::encode_bundle(&id.public, s, &spk.public, 1, &[7u8; qt_core::identity::PQ_PUBLIC_KEY_BYTES])
        .expect("bundle")
}

/// Conta registada (com prova de trabalho). Devolve a ligação e o tempo da PoW + registo.
async fn conta(url: &str, pin: &Option<Vec<u8>>) -> Result<(SigningKeyPair, RelayClient, Duration), String> {
    let s = SigningKeyPair::generate();
    let mut c = ligar(url, pin, &s).await?;
    let t = Instant::now();
    c.register_bundle(bundle(&s)).await.map_err(|e| format!("registo: {e:?}"))?;
    Ok((s, c, t.elapsed()))
}

/// Duas contas normais trocam uma mensagem. `true` se chegou.
async fn servico_normal(url: &str, pin: &Option<Vec<u8>>) -> Result<(), String> {
    let (_, mut a, _) = conta(url, pin).await?;
    let (_, mut b, _) = conta(url, pin).await?;
    let to = b.user_id().to_string();
    a.send_message(&to, vec![], b"ola".to_vec()).await.map_err(|e| format!("envio: {e:?}"))?;
    let got = b.fetch_messages().await.map_err(|e| format!("leitura: {e:?}"))?;
    let ok = got.iter().any(|m| m.ciphertext == b"ola");
    let ids: Vec<u64> = got.iter().map(|m| m.id).collect();
    let _ = b.ack(ids).await;
    let _ = a.delete_account().await;
    let _ = b.delete_account().await;
    if ok { Ok(()) } else { Err("a mensagem nao chegou".into()) }
}

fn conta_resposta(r: &BTreeMap<String, u64>) -> String {
    r.iter().map(|(k, v)| format!("{v} x \"{k}\"")).collect::<Vec<_>>().join(", ")
}

#[tokio::main]
async fn main() {
    let url = arg("--relay", "ws://127.0.0.1:9550");
    let pin = Some(arg("--pin", "")).filter(|p| !p.is_empty()).map(|p| unhex(&p));
    let modo = arg("--modo", "descartaveis");
    let local = url.starts_with("ws://127.0.0.1") || url.starts_with("ws://localhost");
    let mut ligacoes: usize = arg("--ligacoes", "20").parse().expect("--ligacoes");
    let mut msgs: usize = arg("--msgs", "300").parse().expect("--msgs");
    let contas: usize = arg("--contas", "10").parse().expect("--contas");
    let tamanho: usize = arg("--tamanho", "14000").parse().expect("--tamanho");
    if !local {
        ligacoes = ligacoes.min(1);
        msgs = msgs.min(10);
        println!("Relay remoto: limitado a {ligacoes} ligacao e {msgs} mensagens.");
    }

    let antes = servico_normal(&url, &pin).await;
    println!("SERVICO NORMAL (antes): {}", antes.as_ref().map(|_| "OK".to_string()).unwrap_or_else(|e| format!("FALHOU ({e})")));

    let respostas = Arc::new(Mutex::new(BTreeMap::<String, u64>::new()));
    let aceites = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let t0 = Instant::now();
    match modo.as_str() {
        "descartaveis" => {
            let mut tarefas = Vec::new();
            for _ in 0..ligacoes {
                let (url, pin, respostas, aceites) = (url.clone(), pin.clone(), respostas.clone(), aceites.clone());
                tarefas.push(tokio::spawn(async move {
                    let s = SigningKeyPair::generate();
                    let mut c = match ligar(&url, &pin, &s).await {
                        Ok(c) => c,
                        Err(e) => {
                            *respostas.lock().unwrap().entry(format!("ligacao: {e}")).or_default() += 1;
                            return;
                        }
                    };
                    let alvo = uid(&SigningKeyPair::generate()); // nunca existiu
                    let push = ClientMessage::RegisterPushToken { user_id: uid(&s), token: "x".repeat(4000) };
                    let r = c.request(push).await;
                    let k = match r {
                        Ok(ServerMessage::Ack) => "token aceite".to_string(),
                        Ok(ServerMessage::Error { reason }) => format!("token: {reason}"),
                        other => format!("token: {other:?}"),
                    };
                    *respostas.lock().unwrap().entry(k).or_default() += 1;
                    for _ in 0..msgs {
                        let m = ClientMessage::SendMessage {
                            to: alvo.clone(),
                            sealed_from: vec![0; 1024],
                            ciphertext: vec![9; tamanho],
                            silent: true,
                            wake: None,
                        };
                        let k = match c.request(m).await {
                            Ok(ServerMessage::Ack) => {
                                aceites.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                "aceite".to_string()
                            }
                            Ok(ServerMessage::Error { reason }) => reason,
                            Ok(other) => format!("{other:?}"),
                            Err(e) => format!("erro: {e:?}"),
                        };
                        *respostas.lock().unwrap().entry(k).or_default() += 1;
                    }
                }));
            }
            for t in tarefas {
                let _ = t.await;
            }
        }
        "contas" => {
            let mut cs = Vec::new();
            let mut pow = Duration::ZERO;
            for _ in 0..contas {
                match conta(&url, &pin).await {
                    Ok((_, c, d)) => {
                        pow += d;
                        cs.push(c);
                    }
                    Err(e) => println!("conta falhou: {e}"),
                }
            }
            let n = cs.len().max(1);
            println!("{} contas registadas; prova de trabalho + registo: {:.2} s por conta", cs.len(), pow.as_secs_f64() / n as f64);
            let ids: Vec<String> = cs.iter().map(|c| c.user_id().to_string()).collect();
            let mut tarefas = Vec::new();
            for (i, mut c) in cs.into_iter().enumerate() {
                let (to, respostas, aceites) = (ids[(i + 1) % ids.len()].clone(), respostas.clone(), aceites.clone());
                tarefas.push(tokio::spawn(async move {
                    for _ in 0..msgs {
                        let m = ClientMessage::SendMessage { to: to.clone(), sealed_from: vec![0; 1024], ciphertext: vec![9; tamanho], silent: true, wake: None };
                        let k = match c.request(m).await {
                            Ok(ServerMessage::Ack) => {
                                aceites.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                "aceite".to_string()
                            }
                            Ok(ServerMessage::Error { reason }) => reason,
                            Ok(other) => format!("{other:?}"),
                            Err(e) => format!("erro: {e:?}"),
                        };
                        *respostas.lock().unwrap().entry(k).or_default() += 1;
                    }
                    c
                }));
            }
            let mut fim = Vec::new();
            for t in tarefas {
                if let Ok(c) = t.await {
                    fim.push(c);
                }
            }
            // Limpeza: apagar a conta apaga a fila de cada uma.
            for mut c in fim {
                let _ = c.delete_account().await;
            }
        }
        _ => panic!("--modo descartaveis | contas"),
    }
    let dt = t0.elapsed().as_secs_f64();
    let a = aceites.load(std::sync::atomic::Ordering::Relaxed);
    let mb = a as f64 * (tamanho + 1024) as f64 / 1e6;
    println!("Respostas: {}", conta_resposta(&respostas.lock().unwrap()));
    println!("Aceites: {a} mensagens, ~{mb:.1} MB guardados no relay em {dt:.1} s (~{:.0} MB/min)", mb / dt * 60.0);

    let depois = servico_normal(&url, &pin).await;
    println!("SERVICO NORMAL (depois): {}", depois.as_ref().map(|_| "OK".to_string()).unwrap_or_else(|e| format!("FALHOU ({e})")));
}
