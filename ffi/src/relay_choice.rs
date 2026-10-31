//! CONTA NOVA NO RELAY MAIS PROXIMO (0.9.69).
//!
//! Ao criar uma conta (e na recuperacao com as 12 palavras, quando a conta tem de ser criada de
//! novo), a app pergunta ao nucleo por que ordem tentar os relays da sua lista:
//!
//!   * `relay_order_for_new_account` mede o tempo de resposta de cada relay e devolve os nomes do
//!     mais rapido para o mais lento. Os que nao respondem ficam no fim, pela ordem da lista; sem
//!     nenhuma resposta, fica a ordem da lista.
//!   * `relay_next_for_new_account` da o primeiro dessa ordem que ainda nao foi tentado: a app
//!     regista-se nele e, se ele responder "relay sem contas novas", passa ao seguinte.
//!
//! A MEDIDA e so a ligacao TCP a porta do relay (o mesmo endereco e porta das apps), aberta e
//! fechada logo: sem TLS, sem pedidos, sem nada do protocolo. O relay e o HAProxy nao mudam e nao
//! ficam a saber nada da conta (so que este endereco IP abriu e fechou uma ligacao). Duas medidas
//! por relay, em paralelo, e fica a menor: a primeira pode apanhar o acordar do radio.
//!
//! Nunca depois: as contas que ja existem ficam no relay onde estao (a app so pergunta isto quando
//! vai criar uma conta). Nada disto aparece no ecra.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Um relay da lista das apps.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RelayCandidate {
    pub name: String,
    /// O URL das apps (`wss://host[:porta]`).
    pub url: String,
}

/// Medidas por relay (a menor fica).
const ROUNDS: usize = 2;
/// Tempo maximo, mesmo que a app peca mais.
const MAX_TIMEOUT_MS: u32 = 10_000;

/// Host e porta de um URL `wss://` ou `ws://` (porta por omissao 443 ou 80).
fn host_port(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let default = match scheme {
        "wss" => 443,
        "ws" => 80,
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6.split_once(']')?;
        (h, after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return None;
    }
    let port = match port {
        Some(p) => p.parse().ok()?,
        None => default,
    };
    Some((host.to_string(), port))
}

/// Tempo de uma ligacao TCP ao relay (a menor de `ROUNDS`), ou `None` se nao respondeu ate
/// `deadline` (nome que nao resolve, ligacao recusada, rede que nao chega).
fn probe(url: &str, deadline: Instant) -> Option<Duration> {
    let (host, port) = host_port(url)?;
    let addrs: Vec<SocketAddr> = (host.as_str(), port).to_socket_addrs().ok()?.collect();
    let mut best: Option<Duration> = None;
    for _ in 0..ROUNDS {
        let mut got = None;
        for a in &addrs {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return best;
            }
            let t = Instant::now();
            if let Ok(s) = TcpStream::connect_timeout(a, left) {
                got = Some(t.elapsed());
                drop(s);
                break;
            }
        }
        match got {
            Some(d) => best = Some(best.map_or(d, |b| b.min(d))),
            // Nao respondeu: nao vale a pena repetir.
            None => break,
        }
    }
    best
}

/// A ordem a tentar (indices da lista): os que responderam, do mais rapido para o mais lento, e
/// depois os outros pela ordem da lista. Empates: a ordem da lista.
pub(crate) fn order_by_rtt(rtts: &[Option<Duration>]) -> Vec<usize> {
    let mut measured: Vec<(Duration, usize)> = rtts.iter().enumerate().filter_map(|(i, r)| r.map(|d| (d, i))).collect();
    measured.sort();
    let mut out: Vec<usize> = measured.into_iter().map(|(_, i)| i).collect();
    out.extend(rtts.iter().enumerate().filter(|(_, r)| r.is_none()).map(|(i, _)| i));
    out
}

/// Mede todos ao mesmo tempo, ate `timeout` no total. Uma medida que chegue depois e ignorada.
fn measure_all(relays: &[RelayCandidate], timeout: Duration) -> Vec<Option<Duration>> {
    let deadline = Instant::now() + timeout;
    let (tx, rx) = mpsc::channel();
    for (i, r) in relays.iter().enumerate() {
        let (txi, url) = (tx.clone(), r.url.clone());
        let spawned = std::thread::Builder::new().name("qt-relay-probe".into()).spawn(move || {
            let _ = txi.send((i, probe(&url, deadline)));
        });
        if spawned.is_err() {
            let _ = tx.send((i, None));
        }
    }
    drop(tx);
    let mut rtts = vec![None; relays.len()];
    let mut left = relays.len();
    while left > 0 {
        let wait = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok((i, r)) => {
                rtts[i] = r;
                left -= 1;
            }
            Err(_) => break,
        }
    }
    rtts
}

/// CONTA NOVA: por que ordem tentar os relays da lista (nomes, do mais rapido para o mais lento;
/// os que nao responderem no fim, pela ordem da lista). Sem nenhuma resposta: a ordem da lista.
/// Bloqueia ate `timeout_ms` (no maximo 10 s): chamar fora da thread principal.
#[uniffi::export]
pub fn relay_order_for_new_account(relays: Vec<RelayCandidate>, timeout_ms: u32) -> Vec<String> {
    let timeout = Duration::from_millis(timeout_ms.clamp(1, MAX_TIMEOUT_MS) as u64);
    let rtts = measure_all(&relays, timeout);
    order_by_rtt(&rtts).into_iter().map(|i| relays[i].name.clone()).collect()
}

/// CONTA NOVA: o relay onde tentar a seguir, o primeiro de `order` que ainda nao esta em `tried`
/// (os que responderam "relay sem contas novas"). `None`: ja se tentaram todos.
#[uniffi::export]
pub fn relay_next_for_new_account(order: Vec<String>, tried: Vec<String>) -> Option<String> {
    order.into_iter().find(|n| !tried.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn ms(n: u64) -> Option<Duration> {
        Some(Duration::from_millis(n))
    }

    fn cand(name: &str, url: String) -> RelayCandidate {
        RelayCandidate { name: name.into(), url }
    }

    /// Uma porta local que ninguem ouve (ligacao recusada: "nao responde").
    fn closed_port() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }

    #[test]
    fn host_e_porta_do_url() {
        assert_eq!(host_port("wss://relay.sealerapp.com"), Some(("relay.sealerapp.com".into(), 443)));
        assert_eq!(host_port("wss://relay-es.sealerapp.com/"), Some(("relay-es.sealerapp.com".into(), 443)));
        assert_eq!(host_port("wss://13-51-46-213.sslip.io:8443/x"), Some(("13-51-46-213.sslip.io".into(), 8443)));
        assert_eq!(host_port("ws://127.0.0.1:9"), Some(("127.0.0.1".into(), 9)));
        assert_eq!(host_port("ws://[::1]:9443"), Some(("::1".into(), 9443)));
        assert_eq!(host_port("wss://[2001:db8::1]"), Some(("2001:db8::1".into(), 443)));
        for bad in ["https://x", "nada", "wss://", "wss://:443", "wss://x:porta"] {
            assert_eq!(host_port(bad), None, "{bad}");
        }
    }

    /// O mais rapido primeiro; os que nao responderam no fim, pela ordem da lista; sem nenhuma
    /// resposta, a ordem da lista.
    #[test]
    fn o_mais_rapido_primeiro() {
        // Estocolmo 48 ms, Espanha 21 ms: Espanha primeiro.
        assert_eq!(order_by_rtt(&[ms(48), ms(21)]), vec![1, 0]);
        // O primeiro da lista nao responde: fica no fim.
        assert_eq!(order_by_rtt(&[None, ms(60), ms(30)]), vec![2, 1, 0]);
        assert_eq!(order_by_rtt(&[None, ms(30), None, ms(10)]), vec![3, 1, 0, 2]);
        // Nenhum respondeu: a ordem da lista.
        assert_eq!(order_by_rtt(&[None, None, None]), vec![0, 1, 2]);
        // Empate: a ordem da lista.
        assert_eq!(order_by_rtt(&[ms(20), ms(20)]), vec![0, 1]);
        assert!(order_by_rtt(&[]).is_empty());
    }

    /// O primeiro da ordem fechado a contas novas: o seguinte; todos tentados: nenhum.
    #[test]
    fn fechado_passa_ao_seguinte() {
        let order: Vec<String> = ["espanha", "estocolmo"].map(String::from).to_vec();
        assert_eq!(relay_next_for_new_account(order.clone(), vec![]), Some("espanha".into()));
        assert_eq!(relay_next_for_new_account(order.clone(), vec!["espanha".into()]), Some("estocolmo".into()));
        assert_eq!(relay_next_for_new_account(order, vec!["espanha".into(), "estocolmo".into()]), None);
    }

    /// Medida a serio: o relay que responde fica a frente dos que recusam a ligacao ou nao
    /// resolvem, mesmo estando depois na lista; sem nenhum a responder, a ordem da lista.
    #[test]
    fn quem_nao_responde_fica_no_fim() {
        let ouvinte = TcpListener::bind("127.0.0.1:0").unwrap();
        let vivo = format!("ws://127.0.0.1:{}", ouvinte.local_addr().unwrap().port());
        let recusa = format!("ws://127.0.0.1:{}", closed_port());
        let relays = vec![
            cand("recusa", recusa.clone()),
            cand("sem-nome", "wss://nao-existe.invalid".into()),
            cand("vivo", vivo),
        ];
        let t = Instant::now();
        assert_eq!(relay_order_for_new_account(relays, 2_000), vec!["vivo", "recusa", "sem-nome"]);
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        let ninguem = vec![cand("a", recusa), cand("b", "wss://nao-existe.invalid".into())];
        assert_eq!(relay_order_for_new_account(ninguem, 500), vec!["a", "b"]);
        assert!(probe(&format!("ws://127.0.0.1:{}", ouvinte.local_addr().unwrap().port()), Instant::now() + Duration::from_secs(1)).is_some());
    }

    /// Um relay que nunca responde (pacotes perdidos) nao atrasa a resposta para la do prazo.
    #[test]
    fn prazo_cumprido_com_relay_mudo() {
        let ouvinte = TcpListener::bind("127.0.0.1:0").unwrap();
        let vivo = format!("ws://127.0.0.1:{}", ouvinte.local_addr().unwrap().port());
        // 192.0.2.0/24 (TEST-NET-1): nunca responde (ou a rede diz logo que nao chega).
        let relays = vec![cand("mudo", "wss://192.0.2.1".into()), cand("vivo", vivo)];
        let t = Instant::now();
        assert_eq!(relay_order_for_new_account(relays, 800), vec!["vivo", "mudo"]);
        assert!(t.elapsed() < Duration::from_millis(1_500), "{:?}", t.elapsed());
    }
}
