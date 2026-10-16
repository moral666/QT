//! Estatisticas ANONIMAS do relay: so contagens, sem IDs, IPs, chaves nem
//! conteudo. Servem para ver o estado do servico (ex.: "ha envios mas nenhuma
//! entrega") sem criar um registo de metadados de quem fala com quem.
//!
//! O binario `relay_server` escreve uma linha por minuto com os valores do
//! ultimo minuto (`sudo journalctl -u qt-relay -f`).

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[derive(Default)]
pub struct Stats {
    /// Ligacoes abertas neste momento (nao e reposto a cada minuto).
    pub active: AtomicU64,
    pub connections: AtomicU64,
    pub rejected_busy: AtomicU64,
    pub handshake_failed: AtomicU64,
    pub auth_ok: AtomicU64,
    pub auth_failed: AtomicU64,
    pub bundles_registered: AtomicU64,
    pub bundles_fetched: AtomicU64,
    pub bundles_not_found: AtomicU64,
    pub sends: AtomicU64,
    pub queue_full: AtomicU64,
    pub fetches: AtomicU64,
    /// Pedidos `WaitMessages` que ficaram a espera (fila vazia no momento do pedido).
    pub waits: AtomicU64,
    pub delivered: AtomicU64,
    pub acked: AtomicU64,
    pub invalid_requests: AtomicU64,
    pub internal_errors: AtomicU64,
    pub blob_uploads: AtomicU64,
    pub blob_downloads: AtomicU64,
}

pub static STATS: Stats = Stats {
    active: AtomicU64::new(0),
    connections: AtomicU64::new(0),
    rejected_busy: AtomicU64::new(0),
    handshake_failed: AtomicU64::new(0),
    auth_ok: AtomicU64::new(0),
    auth_failed: AtomicU64::new(0),
    bundles_registered: AtomicU64::new(0),
    bundles_fetched: AtomicU64::new(0),
    bundles_not_found: AtomicU64::new(0),
    sends: AtomicU64::new(0),
    queue_full: AtomicU64::new(0),
    fetches: AtomicU64::new(0),
    waits: AtomicU64::new(0),
    delivered: AtomicU64::new(0),
    acked: AtomicU64::new(0),
    invalid_requests: AtomicU64::new(0),
    internal_errors: AtomicU64::new(0),
    blob_uploads: AtomicU64::new(0),
    blob_downloads: AtomicU64::new(0),
};

pub fn inc(c: &AtomicU64) {
    c.fetch_add(1, Relaxed);
}

pub fn add(c: &AtomicU64, n: u64) {
    c.fetch_add(n, Relaxed);
}

/// Linha do ultimo intervalo (e repoe os contadores do intervalo). `None` se
/// nao houve atividade nenhuma (para nao encher o registo sem razao).
pub fn take_line(s: &Stats) -> Option<String> {
    let t = |c: &AtomicU64| c.swap(0, Relaxed);
    let v = [
        ("ligacoes", t(&s.connections)),
        ("ocupado", t(&s.rejected_busy)),
        ("handshake_falhou", t(&s.handshake_failed)),
        ("auth_ok", t(&s.auth_ok)),
        ("auth_falhou", t(&s.auth_failed)),
        ("bundles_publicados", t(&s.bundles_registered)),
        ("bundles_pedidos", t(&s.bundles_fetched)),
        ("bundle_nao_encontrado", t(&s.bundles_not_found)),
        ("envios", t(&s.sends)),
        ("fila_cheia", t(&s.queue_full)),
        ("pedidos_de_mensagens", t(&s.fetches)),
        ("esperas", t(&s.waits)),
        ("mensagens_entregues", t(&s.delivered)),
        ("confirmadas", t(&s.acked)),
        ("pedidos_invalidos", t(&s.invalid_requests)),
        ("erros_internos", t(&s.internal_errors)),
        ("anexos_enviados", t(&s.blob_uploads)),
        ("anexos_descarregados", t(&s.blob_downloads)),
    ];
    let active = s.active.load(Relaxed);
    if active == 0 && v.iter().all(|(_, n)| *n == 0) {
        return None;
    }
    let body: Vec<String> = v.iter().map(|(k, n)| format!("{k}={n}")).collect();
    Some(format!("stats 60s: ativas={active} {}", body.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linha_so_com_contagens_e_repoe_o_intervalo() {
        let s = Stats::default();
        assert!(take_line(&s).is_none(), "sem atividade nao escreve nada");
        inc(&s.sends);
        add(&s.delivered, 3);
        let l = take_line(&s).unwrap();
        assert!(l.contains("envios=1") && l.contains("mensagens_entregues=3"), "{l}");
        assert!(take_line(&s).is_none(), "os contadores do intervalo foram repostos");
    }
}
