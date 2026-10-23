//! Fichas anonimas do video pelo relay (patch 0006), do lado dos servidores. A criptografia
//! esta em `qt_core::fichas` (crate `voprf`); aqui ficam a semente, o registo das gastas e
//! a contagem das emitidas.
//!
//!   * SEMENTE (`/var/lib/qt/fichas_seed.bin`, 32 bytes, 0600, dono `qt`): gerada uma vez pelo
//!     relay das mensagens (o emissor). O relay das chamadas so a le. Cada mes tem a sua chave,
//!     derivada dela (`Issuer::derive`): rotacao sem trocar ficheiros.
//!   * EMITIDAS (relay das mensagens, Redis `default:fichas:emitidas:<conta>:<AAAAMM>`): so um
//!     numero por conta e por mes, apagado ~40 dias depois. Nunca se guarda o pedido nem a
//!     resposta.
//!   * GASTAS (relay das chamadas, Redis `fichas:gastas:<AAAAMM>:<resumo>`): so o resumo do
//!     nonce, com prazo ate ao fim do mes mais um dia. Nada que diga que conta a pediu.

use std::sync::{Arc, Mutex};

use qt_core::fichas::Issuer;
use zeroize::Zeroizing;

/// Gera ou le a semente das chaves das fichas. So o relay das mensagens a cria.
pub fn load_or_create_seed(path: &std::path::Path) -> std::io::Result<Zeroizing<[u8; 32]>> {
    match load_seed(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut seed = Zeroizing::new([0u8; 32]);
            getrandom_fill(&mut seed[..])?;
            write_private_file(path, &seed[..])?;
            Ok(seed)
        }
        Err(e) => Err(e),
    }
}

/// Le a semente (o relay das chamadas nunca a cria).
pub fn load_seed(path: &std::path::Path) -> std::io::Result<Zeroizing<[u8; 32]>> {
    let bytes = Zeroizing::new(std::fs::read(path)?);
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "semente das fichas com tamanho errado"))?;
    Ok(Zeroizing::new(arr))
}

fn getrandom_fill(buf: &mut [u8]) -> std::io::Result<()> {
    use rand::RngCore;
    rand::rngs::OsRng.try_fill_bytes(buf).map_err(std::io::Error::other)
}

/// Ficheiro com permissoes 0600 desde a criacao.
fn write_private_file(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(data)
}

/// Chaves do emissor (relay das mensagens): a do mes pedido, derivada uma vez e guardada.
pub struct IssuerKeys {
    seed: Zeroizing<[u8; 32]>,
    cache: Mutex<Option<Arc<Issuer>>>,
}

impl std::fmt::Debug for IssuerKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IssuerKeys(..)")
    }
}

impl IssuerKeys {
    pub fn new(seed: [u8; 32]) -> Self {
        IssuerKeys { seed: Zeroizing::new(seed), cache: Mutex::new(None) }
    }

    pub fn for_epoch(&self, epoch: u32) -> Option<Arc<Issuer>> {
        let mut c = self.cache.lock().ok()?;
        if c.as_ref().is_none_or(|i| i.epoch() != epoch) {
            *c = Some(Arc::new(Issuer::derive(&self.seed, epoch).ok()?));
        }
        c.clone()
    }
}

/// Chaves publicas das fichas de `months` meses a comecar em `from`, para o site
/// (`https://sealer.pt/estado.json`, fora do relay): `{"versao":1,"fichas":{"AAAAMM":"hex",...}}`.
/// As apps so emitem e gastam fichas cuja chave esteja aqui: um relay comprometido nao
/// consegue dar a cada conta uma chave diferente (particao de chaves) sem mudar o site.
pub fn published_keys_json(seed: &[u8; 32], from: u32, months: u32) -> String {
    let mut map = serde_json::Map::new();
    let mut e = from;
    for _ in 0..months {
        if let Ok(i) = Issuer::derive(seed, e) {
            let hex: String = i.public_key().iter().map(|b| format!("{b:02x}")).collect();
            map.insert(format!("{e:06}"), serde_json::Value::String(hex));
        }
        e = qt_core::fichas::next_epoch(e);
    }
    let mut out = serde_json::to_string_pretty(&serde_json::json!({ "versao": 1, "fichas": map })).unwrap_or_default();
    out.push('\n');
    out
}

/// Registo das fichas gastas (relay das chamadas), no Redis.
pub struct SpentStore {
    client: redis::Client,
    shared: tokio::sync::OnceCell<redis::aio::ConnectionManager>,
    prefix: String,
}

impl SpentStore {
    /// `url`: o do utilizador `qt-fichas` (so `SET` nas chaves `fichas:*`).
    pub fn new(url: &str) -> Result<Self, redis::RedisError> {
        Self::with_prefix(url, "fichas")
    }

    pub fn with_prefix(url: &str, prefix: &str) -> Result<Self, redis::RedisError> {
        Ok(SpentStore { client: redis::Client::open(url)?, shared: tokio::sync::OnceCell::new(), prefix: prefix.to_string() })
    }

    /// Marca a ficha como gasta. `true`: primeira vez; `false`: ja estava gasta.
    pub async fn mark(&self, epoch: u32, spent_id: &[u8; 16]) -> Result<bool, redis::RedisError> {
        let mut conn = self.shared.get_or_try_init(|| redis::aio::ConnectionManager::new(self.client.clone())).await?.clone();
        let hex: String = spent_id.iter().map(|b| format!("{b:02x}")).collect();
        let key = format!("{}:gastas:{epoch}:{hex}", self.prefix);
        // Ate ao fim do mes da ficha e mais um dia (depois disso ja nao valeria de qualquer forma).
        let ttl = qt_core::fichas::epoch_end_unix(epoch).saturating_sub(qt_core::fichas::unix_now()) + 86_400;
        let r: Option<String> = redis::cmd("SET").arg(&key).arg(1u8).arg("NX").arg("EX").arg(ttl).query_async(&mut conn).await?;
        Ok(r.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semente_criada_uma_vez_e_lida_depois() {
        let dir = std::env::temp_dir().join(format!("qt-fichas-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("fichas_seed.bin");
        assert!(load_seed(&p).is_err(), "o relay das chamadas nao cria");
        let a = load_or_create_seed(&p).unwrap();
        let b = load_or_create_seed(&p).unwrap();
        assert_eq!(*a, *b);
        assert_eq!(*load_seed(&p).unwrap(), *a);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::write(dir.join("curta"), [1u8; 5]).unwrap();
        assert!(load_seed(&dir.join("curta")).is_err());
        let keys = IssuerKeys::new(*a);
        let k1 = keys.for_epoch(202610).unwrap();
        assert_eq!(k1.public_key(), Issuer::derive(&a, 202610).unwrap().public_key());
        assert_ne!(keys.for_epoch(202611).unwrap().public_key(), k1.public_key());
        // Ficheiro do site: 24 meses seguidos, a passar o fim do ano.
        let j: serde_json::Value = serde_json::from_str(&published_keys_json(&a, 202611, 24)).unwrap();
        let f = j["fichas"].as_object().unwrap();
        assert_eq!(f.len(), 24);
        assert!(f.contains_key("202612") && f.contains_key("202701") && f.contains_key("202810"));
        let hex: String = k1.public_key().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(j["fichas"]["202610"], serde_json::Value::Null, "comeca no mes pedido");
        let j2: serde_json::Value = serde_json::from_str(&published_keys_json(&a, 202610, 1)).unwrap();
        assert_eq!(j2["fichas"]["202610"].as_str(), Some(hex.as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
