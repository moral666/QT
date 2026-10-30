//! FERRAMENTA DO OPERADOR: COMUNIDADE (0.9.52; so no computador do operador, chamada pelo
//! Sealer Control no botao "Atualizar Comunidade").
//!
//! ```text
//! ... --example comunidade -- recolher --key operador.key --estado comunidade.estado \
//!         --registo badge-emissor.db.registo --relays "wss://...|<pin>;wss://...|<pin>" [--json]
//! ... --example comunidade -- publicar --key operador.key --emissor badge-emissor.key \
//!         --estado comunidade.estado --registo badge-emissor.db.registo --ocultos comunidade.json \
//!         --relays "wss://...|<pin>;wss://...|<pin>" [--json]
//! ```
//!
//! `recolher`: busca os envios das apps em todos os relays, abre-os com a chave do operador e
//! confere cada um: certificado do emissor, autenticacao da identidade do certificado (um
//! certificado copiado de outra pessoa e recusado) e o registo das medalhas (`badge` escreve
//! `<db>.registo`: uma medalha retirada ou substituida depois deixa de contar). Os validos ficam
//! no ESTADO (ficheiro cifrado com a chave do operador, so neste PC); os recusados sao apagados
//! logo do relay. Mostra os membros (o operador pode marcar "nao mostrar").
//!
//! `publicar`: monta a lista com todos os membros com a medalha em dia, menos os marcados
//! "nao mostrar" (ficheiro `--ocultos`: `{"<membro>": {"esconder": true}}`). Cada membro leva
//! so a medalha e a foto, sem nome nem ID, pela ordem das medalhas e, dentro de cada uma, pela
//! ordem de entrada (a primeira vez que o envio dele chegou). Assina-a com a chave das medalhas,
//! cifra-a e envia-a, com as fotos que faltam, a TODOS os relays (a mesma lista). So depois
//! apaga os envios tratados. A mesma foto mantem o mesmo hash de semana para semana (as apps
//! nao a voltam a descarregar).
//!
//! A chave das medalhas e a do operador nunca saem deste PC: o relay so ve bytes cifrados.

use base64::Engine;
use qt_core::{badge, community, report};
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::protocol::{ClientMessage, ServerMessage, MAX_COMMUNITY_KEEP};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use zeroize::Zeroizing;

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned())
}

fn flag(name: &str) -> bool {
    std::env::args().any(|x| x == name)
}

fn expand(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(p),
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("ERRO: {msg}");
    std::process::exit(1)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn seed_from(opt: &str, what: &str) -> Zeroizing<[u8; 32]> {
    let path = expand(&arg(opt).unwrap_or_else(|| fail(&format!("falta {opt} <ficheiro da {what}>"))));
    let text = Zeroizing::new(std::fs::read_to_string(&path).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display()))));
    let bytes = Zeroizing::new(unhex(&text).unwrap_or_else(|| fail(&format!("{what} estragada"))));
    let seed: [u8; 32] = bytes.as_slice().try_into().unwrap_or_else(|_| fail(&format!("{what} com tamanho errado")));
    Zeroizing::new(seed)
}

// ------------------------------------------------------------------ estado (so neste PC)

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Membro {
    medalha: u8,
    /// Quando entrou na Comunidade (a hora a que o relay recebeu o primeiro envio valido): a
    /// ordem dentro da medalha, a mesma qualquer que seja o relay.
    #[serde(default)]
    entrou_em_ms: u64,
    /// Certificado (hex).
    cert: String,
    emitida_em_ms: i64,
    enviada_em_ms: i64,
    /// A foto como a app a enviou (base64); `None` = sem foto.
    foto: Option<String>,
    /// A foto ja cifrada para a lista e o seu hash (o mesmo enquanto a foto nao mudar).
    foto_hash: Option<String>,
    foto_blob: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct Estado {
    /// Numero da ultima lista publicada.
    versao: u64,
    publicada_em_ms: u64,
    membros: BTreeMap<String, Membro>,
    /// Envios ja guardados no estado, por relay (`url`): apagados depois de publicar.
    tratados: BTreeMap<String, Vec<u64>>,
}

fn estado_path() -> PathBuf {
    expand(&arg("--estado").unwrap_or_else(|| fail("falta --estado <ficheiro do estado da comunidade>")))
}

fn load_state(seed: &[u8; 32]) -> Estado {
    let path = estado_path();
    match std::fs::read(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Estado::default(),
        Err(e) => fail(&format!("{}: {e}", path.display())),
        Ok(blob) => {
            let plain = Zeroizing::new(
                community::open_state(seed, &blob).unwrap_or_else(|_| fail("estado da comunidade estragado ou de outra chave do operador")),
            );
            serde_json::from_slice(&plain).unwrap_or_else(|e| fail(&format!("estado da comunidade: {e}")))
        }
    }
}

fn save_state(seed: &[u8; 32], st: &Estado) {
    use std::io::Write;
    let path = estado_path();
    let plain = Zeroizing::new(serde_json::to_vec(st).expect("JSON"));
    let blob = community::seal_state(seed, &plain);
    // Escreve ao lado e troca: um erro a meio nunca estraga o estado anterior.
    let tmp = path.with_extension("tmp");
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(&tmp).unwrap_or_else(|e| fail(&format!("{}: {e}", tmp.display())));
    f.write_all(&blob).and_then(|_| f.sync_all()).unwrap_or_else(|e| fail(&e.to_string()));
    std::fs::rename(&tmp, &path).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())));
}

// ------------------------------------------------------------------ registo das medalhas

#[derive(Deserialize, Clone)]
struct Registo {
    membro: String,
    #[serde(default)]
    id: String,
    badge: u8,
    emitido_em_ms: i64,
}

/// O ultimo badge entregue a cada membro (ferramenta `badge`). Sem ficheiro: vazio.
fn load_registry() -> BTreeMap<String, Registo> {
    let mut out: BTreeMap<String, Registo> = BTreeMap::new();
    let Some(path) = arg("--registo").map(|p| expand(&p)) else { return out };
    let Ok(text) = std::fs::read_to_string(&path) else { return out };
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(r) = serde_json::from_str::<Registo>(line) else { continue };
        if out.get(&r.membro).is_none_or(|old| old.emitido_em_ms <= r.emitido_em_ms) {
            out.insert(r.membro.clone(), r);
        }
    }
    out
}

/// "ok", "retirada" (o ultimo badge entregue e "nenhum") ou "substituida" (ha um badge mais
/// recente que a app ainda nao enviou).
fn estado_medalha(m: &Membro, reg: Option<&Registo>) -> &'static str {
    match reg {
        Some(r) if r.emitido_em_ms >= m.emitida_em_ms && r.badge == badge::BADGE_NONE => "retirada",
        Some(r) if r.emitido_em_ms > m.emitida_em_ms && r.badge != m.medalha => "substituida",
        _ => "ok",
    }
}

// ------------------------------------------------------------------ relays

struct Relay {
    url: String,
    pin: String,
}

fn relays() -> Vec<Relay> {
    let raw = arg("--relays").unwrap_or_else(|| fail("falta --relays \"wss://...|<pin>;wss://...|<pin>\""));
    let list: Vec<Relay> = raw
        .split(';')
        .filter(|e| !e.trim().is_empty())
        .map(|e| {
            let (url, pin) = e.trim().split_once('|').unwrap_or_else(|| fail(&format!("relay invalido: {e}")));
            let ok_pin = pin.is_empty() || (pin.len() == 64 && unhex(pin).is_some());
            if !(url.starts_with("wss://") || url.starts_with("ws://")) || !ok_pin {
                fail(&format!("relay invalido: {e}"));
            }
            Relay { url: url.to_string(), pin: pin.to_lowercase() }
        })
        .collect();
    if list.is_empty() {
        fail("--relays vazio");
    }
    list
}

async fn connect(seed: &[u8; 32], r: &Relay) -> Result<RelayClient, String> {
    let noise = qt_transport::generate_static_keypair().map_err(|e| format!("{e:?}"))?;
    let cfg = RelayConfig {
        expected_server_static: (!r.pin.is_empty()).then(|| unhex(&r.pin).unwrap_or_default()),
        max_retries: 1,
        ..RelayConfig::default()
    };
    let signing = report::operator_signing_private(seed);
    RelayClient::connect(&r.url, noise.private, *signing, cfg).await.map_err(|e| format!("ligacao ao relay: {e}"))
}

async fn ask(c: &mut RelayClient, m: ClientMessage) -> Result<ServerMessage, String> {
    match c.request(m).await {
        Ok(ServerMessage::Error { reason }) => Err(format!("o relay recusou: {reason}")),
        Ok(r) => Ok(r),
        Err(e) => Err(format!("relay: {e}")),
    }
}

// ------------------------------------------------------------------ recolher

/// Junta um envio valido ao estado: vale o certificado mais recente e, com o mesmo, o envio
/// mais recente. `recebido_ms`: quando o relay o recebeu (a entrada de quem e novo). `true` se
/// mudou alguma coisa.
fn merge(st: &mut Estado, s: community::Submission, recebido_ms: u64) -> bool {
    let key = hex(&s.cert.subject);
    if let Some(old) = st.membros.get(&key) {
        let newer = (s.cert.issued_at, s.sent_at_ms) > (old.emitida_em_ms, old.enviada_em_ms);
        if !newer {
            return false;
        }
    }
    let entrou_em_ms = st.membros.get(&key).map_or(recebido_ms, |old| old.entrou_em_ms);
    let foto = s.photo.as_ref().map(|p| b64().encode(p));
    let same_photo = st.membros.get(&key).is_some_and(|old| old.foto == foto);
    let (foto_hash, foto_blob) = match st.membros.get(&key) {
        Some(old) if same_photo => (old.foto_hash.clone(), old.foto_blob.clone()),
        _ => (None, None),
    };
    st.membros.insert(
        key,
        Membro {
            medalha: s.cert.badge,
            entrou_em_ms,
            cert: hex(&s.cert_bytes),
            emitida_em_ms: s.cert.issued_at,
            enviada_em_ms: s.sent_at_ms,
            foto,
            foto_hash,
            foto_blob,
        },
    );
    true
}

fn issuer_public() -> [u8; 32] {
    // So nos testes da ferramenta: outra chave do emissor (as apps so aceitam a fixa).
    match arg("--emissor-publica") {
        Some(h) => unhex(&h).and_then(|v| v.try_into().ok()).unwrap_or_else(|| fail("--emissor-publica invalida")),
        None => badge::ISSUER_PUBLIC_KEY,
    }
}

async fn cmd_recolher() {
    let seed = seed_from("--key", "chave do operador");
    let issuer = issuer_public();
    let mut st = load_state(&seed);
    let mut novos = 0u32;
    let mut recusados: BTreeMap<String, u32> = BTreeMap::new();
    let mut erros = Vec::new();
    for r in relays() {
        let mut c = match connect(&seed, &r).await {
            Ok(c) => c,
            Err(e) => {
                erros.push(serde_json::json!({"relay": r.url, "erro": e}));
                continue;
            }
        };
        let mut after = 0u64;
        loop {
            let (items, more) = match ask(&mut c, ClientMessage::AdminListCommunity { after }).await {
                Ok(ServerMessage::CommunitySubmissions { items, more }) => (items, more),
                Ok(other) => {
                    erros.push(serde_json::json!({"relay": r.url, "erro": format!("resposta inesperada: {other:?}")}));
                    break;
                }
                Err(e) => {
                    erros.push(serde_json::json!({"relay": r.url, "erro": e}));
                    break;
                }
            };
            for it in &items {
                after = after.max(it.id);
                let opened = b64()
                    .decode(&it.sealed)
                    .map_err(|_| community::CommunityError::BadFormat)
                    .and_then(|b| community::open_submission(&seed, &issuer, &b));
                match opened {
                    Ok(s) => {
                        if merge(&mut st, s, it.received_at_ms) {
                            novos += 1;
                        }
                        let ids = st.tratados.entry(r.url.clone()).or_default();
                        if !ids.contains(&it.id) {
                            ids.push(it.id);
                        }
                    }
                    Err(e) => {
                        let why = match e {
                            community::CommunityError::BadAuth => "certificado de outra pessoa",
                            community::CommunityError::BadCertificate => "certificado invalido",
                            _ => "envio estragado",
                        };
                        *recusados.entry(why.to_string()).or_default() += 1;
                        // Nunca vai ser valido: sai ja do relay.
                        let _ = ask(&mut c, ClientMessage::AdminDeleteCommunity { id: it.id }).await;
                    }
                }
            }
            if !more || items.is_empty() {
                break;
            }
        }
    }
    save_state(&seed, &st);
    let reg = load_registry();
    let membros: Vec<serde_json::Value> = st
        .membros
        .iter()
        .map(|(k, m)| {
            let r = reg.get(k);
            serde_json::json!({
                "membro": k,
                "id": r.map(|r| r.id.clone()).unwrap_or_default(),
                "medalha": m.medalha,
                "entrou_em_ms": m.entrou_em_ms,
                "foto": m.foto,
                "emitida_em_ms": m.emitida_em_ms,
                "enviada_em_ms": m.enviada_em_ms,
                "estado": estado_medalha(m, r),
            })
        })
        .collect();
    let out = serde_json::json!({
        "membros": membros,
        "novos": novos,
        "recusados": recusados,
        "versao": st.versao,
        "publicada_em_ms": st.publicada_em_ms,
        "erros": erros,
    });
    if flag("--json") {
        println!("{out}");
    } else {
        println!("{}", serde_json::to_string_pretty(&out).expect("JSON"));
    }
}

// ------------------------------------------------------------------ publicar

#[derive(Deserialize, Default)]
struct Oculto {
    #[serde(default)]
    esconder: bool,
}

async fn cmd_publicar() {
    let seed = seed_from("--key", "chave do operador");
    let issuer_seed = seed_from("--emissor", "chave do emissor");
    if badge::issuer_public(&issuer_seed) != issuer_public() {
        fail("esta chave do emissor nao e a das apps (ISSUER_PUBLIC_KEY): as apps iam recusar a lista");
    }
    let ocultos: BTreeMap<String, Oculto> = match arg("--ocultos").map(|p| expand(&p)) {
        Some(p) => match std::fs::read_to_string(&p) {
            Ok(t) => serde_json::from_str(&t).unwrap_or_else(|e| fail(&format!("{}: {e}", p.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => fail(&format!("{}: {e}", p.display())),
        },
        None => BTreeMap::new(),
    };
    let reg = load_registry();
    let mut st = load_state(&seed);

    // Os membros da lista: medalha em dia e nao escondidos, por medalha e ordem de entrada.
    let mut order: Vec<(u8, u64, String)> = st
        .membros
        .iter()
        .filter(|(k, m)| estado_medalha(m, reg.get(*k)) == "ok")
        .filter(|(k, _)| !ocultos.get(*k).is_some_and(|o| o.esconder))
        .map(|(k, m)| (m.medalha, m.entrou_em_ms, k.clone()))
        .collect();
    order.sort();
    let ocultos_n = ocultos.values().filter(|o| o.esconder).count();
    let mut members = Vec::new();
    let mut fotos: Vec<(String, Vec<u8>)> = Vec::new();
    for (_, _, k) in order {
        let m = st.membros.get_mut(&k).expect("membro do estado");
        let photo_hash = match &m.foto {
            None => None,
            Some(f) => {
                if m.foto_hash.is_none() || m.foto_blob.is_none() {
                    let raw = b64().decode(f).unwrap_or_else(|_| fail("foto estragada no estado"));
                    let (h, blob) = community::seal_photo(&raw).unwrap_or_else(|_| fail("foto invalida no estado"));
                    m.foto_hash = Some(hex(&h));
                    m.foto_blob = Some(b64().encode(blob));
                }
                let h = m.foto_hash.clone().unwrap_or_default();
                fotos.push((h.clone(), b64().decode(m.foto_blob.as_deref().unwrap_or_default()).unwrap_or_default()));
                Some(unhex(&h).and_then(|v| v.try_into().ok()).unwrap_or_else(|| fail("hash da foto estragado")))
            }
        };
        members.push(community::Member { badge: m.medalha, photo_hash });
    }
    if members.len() > community::MAX_MEMBERS {
        fail(&format!("demasiados membros ({}, no maximo {})", members.len(), community::MAX_MEMBERS));
    }
    let version = (st.versao + 1).max(now_ms() / 1000);
    let ix = community::Index { version, created_ms: now_ms(), members };
    let index = community::build_index(&issuer_seed, &ix).unwrap_or_else(|e| fail(&format!("lista: {e:?}")));
    // O estado com as fotos ja cifradas (mesmo hash na proxima semana) antes de enviar.
    save_state(&seed, &st);

    let mut resultado = Vec::new();
    let mut publicada = false;
    for r in relays() {
        let res: Result<u32, String> = async {
            let mut c = connect(&seed, &r).await?;
            let hashes: Vec<String> = fotos.iter().map(|(h, _)| h.clone()).collect();
            let mut missing = Vec::new();
            for chunk in hashes.chunks(MAX_COMMUNITY_KEEP) {
                match ask(&mut c, ClientMessage::AdminCommunityKeep { hashes: chunk.to_vec() }).await? {
                    ServerMessage::CommunityMissing { hashes } => missing.extend(hashes),
                    other => return Err(format!("resposta inesperada: {other:?}")),
                }
            }
            for (h, blob) in fotos.iter().filter(|(h, _)| missing.contains(h)) {
                ask(&mut c, ClientMessage::AdminCommunityPhoto { hash: h.clone(), data: b64().encode(blob) }).await?;
            }
            ask(&mut c, ClientMessage::AdminCommunityPublish { version, index: b64().encode(&index) }).await?;
            // So agora: os envios tratados saem do relay.
            let done = st.tratados.remove(&r.url).unwrap_or_default();
            for id in done {
                let _ = ask(&mut c, ClientMessage::AdminDeleteCommunity { id }).await;
            }
            Ok(missing.len() as u32)
        }
        .await;
        match res {
            Ok(n) => {
                publicada = true;
                resultado.push(serde_json::json!({"relay": r.url, "ok": true, "fotos_enviadas": n}));
            }
            Err(e) => resultado.push(serde_json::json!({"relay": r.url, "ok": false, "erro": e})),
        }
    }
    if publicada {
        st.versao = version;
        st.publicada_em_ms = now_ms();
    }
    save_state(&seed, &st);
    let out = serde_json::json!({
        "versao": version,
        "membros": ix.members.len(),
        "ocultos": ocultos_n,
        "relays": resultado,
    });
    if flag("--json") {
        println!("{out}");
    } else {
        println!("{}", serde_json::to_string_pretty(&out).expect("JSON"));
    }
    if !publicada {
        std::process::exit(2);
    }
}

#[tokio::main]
async fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("recolher") => cmd_recolher().await,
        Some("publicar") => cmd_publicar().await,
        _ => fail("uso: comunidade recolher|publicar --key ... --estado ... --relays ... (ver o cabecalho)"),
    }
}
