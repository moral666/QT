//! FERRAMENTA DO OPERADOR: DENUNCIAS E BANIMENTOS (so no computador do operador; nao e a app nem o servidor).
//!
//! Passo 1, uma vez: criar a chave do operador (fica num ficheiro, NUNCA no git nem no servidor).
//!
//! ```text
//! cargo run --release -p qt_server --example denuncias -- chave --key ~/Documentos/operador.key
//! ```
//!
//! Escreve duas chaves PUBLICAS: a das denuncias (vai para `OPERATOR_REPORT_PUBLIC` em
//! `core/src/report.rs`, as apps passam a cifrar para ela) e a do relay (vai para
//! `/etc/qt/operator.pub` no servidor; so ela lista denuncias e bane contas).
//!
//! Depois:
//!
//! ```text
//! ... --example denuncias -- listar   --key ~/Documentos/operador.key [--json]
//! ... --example denuncias -- apagar   --key ~/Documentos/operador.key --id 12
//! ... --example denuncias -- banir    --key ~/Documentos/operador.key --user <id do relay>
//! ... --example denuncias -- desbanir --key ~/Documentos/operador.key --user <id do relay>
//! ... --example denuncias -- banidos  --key ~/Documentos/operador.key [--json]
//! ```
//!
//! Premium (0.9.20), com a mesma chave:
//!
//! ```text
//! ... --example denuncias -- premium-dar   --key ... --user qt1:... (--dias 30 | --sempre)
//! ... --example denuncias -- premium-tirar --key ... --user qt1:...
//! ... --example denuncias -- premium-lista --key ... [--json]
//! ... --example denuncias -- premium-bloqueio --key ... [--ligar | --desligar] [--json]
//! ```
//!
//! `premium-bloqueio` (0.9.21): sem opcao so mostra o estado. Ligado, o relay recusa anexos e
//! chamadas a contas sem premium e as apps fecham as funcoes pagas.
//!
//! O relay guarda as denuncias cifradas (nao as consegue ler) e sem o ID de quem denunciou.
//! So esta ferramenta, com a chave, as abre. `--relay` e `--pin` servem para testar noutro relay.

use qt_core::report;
use qt_server::client::{RelayClient, RelayConfig};
use qt_server::protocol::{ClientMessage, ServerMessage, StoredReport};
use std::io::Write;
use std::path::PathBuf;
use zeroize::Zeroizing;

const RELAY: &str = "wss://relay.sealer.pt";
const PIN: &str = "f018d46b67d2437571cefc86827dec0dc3b0fd586bd22c664d3839f6a1c67f1a";

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

fn fail(msg: &str) -> ! {
    eprintln!("ERRO: {msg}");
    std::process::exit(1)
}

fn key_path() -> PathBuf {
    expand(&arg("--key").unwrap_or_else(|| fail("falta --key <ficheiro da chave do operador>")))
}

fn load_seed() -> Zeroizing<[u8; 32]> {
    let path = key_path();
    let text = Zeroizing::new(std::fs::read_to_string(&path).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display()))));
    let bytes = Zeroizing::new(unhex(&text).unwrap_or_else(|| fail("chave do operador estragada")));
    let seed: [u8; 32] = bytes.as_slice().try_into().unwrap_or_else(|_| fail("chave do operador com tamanho errado"));
    Zeroizing::new(seed)
}

fn cmd_chave() {
    let path = key_path();
    if path.exists() {
        fail(&format!("{} ja existe: nao a substituo (as denuncias antigas deixavam de abrir)", path.display()));
    }
    let mut seed = Zeroizing::new([0u8; 32]);
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, seed.as_mut());
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())));
    f.write_all(hex(seed.as_ref()).as_bytes()).unwrap_or_else(|e| fail(&e.to_string()));

    let report_pub = report::operator_report_public(&seed);
    let list: Vec<String> = report_pub.iter().map(|b| format!("0x{b:02x}")).collect();
    println!("Chave do operador criada em {} (guarda uma copia numa pen; nunca no git).", path.display());
    println!();
    println!("1) Apps: em core/src/report.rs,");
    println!("pub const OPERATOR_REPORT_PUBLIC: [u8; 32] = [{}];", list.join(", "));
    println!();
    println!("2) Relay: no servidor,");
    println!("echo {} | sudo tee /etc/qt/operator.pub", hex(&report::operator_signing_public(&seed)));
    println!("e correr o setup-ec2.sh de novo.");
}

async fn connect(seed: &[u8; 32]) -> RelayClient {
    let url = arg("--relay").unwrap_or_else(|| RELAY.to_string());
    let pin = arg("--pin").unwrap_or_else(|| PIN.to_string());
    let noise = qt_transport::generate_static_keypair().unwrap_or_else(|e| fail(&format!("{e:?}")));
    let cfg = RelayConfig {
        expected_server_static: (!pin.is_empty()).then(|| unhex(&pin).unwrap_or_else(|| fail("--pin invalido"))),
        ..RelayConfig::default()
    };
    let signing = report::operator_signing_private(seed);
    RelayClient::connect(&url, noise.private, *signing, cfg)
        .await
        .unwrap_or_else(|e| fail(&format!("ligacao ao relay: {e}")))
}

async fn ask(c: &mut RelayClient, m: ClientMessage) -> ServerMessage {
    match c.request(m).await {
        Ok(ServerMessage::Error { reason }) => fail(&format!("o relay recusou: {reason}")),
        Ok(r) => r,
        Err(e) => fail(&format!("relay: {e}")),
    }
}

fn user_arg() -> String {
    let u = arg("--user").unwrap_or_else(|| fail("falta --user <qt1:... ou id do relay, 64 caracteres hex>"));
    let mut u = u.trim().to_ascii_lowercase();
    // QT Public ID: o id do relay e a segunda metade (chave de assinatura).
    if let Some(rest) = u.strip_prefix("qt1:") {
        if rest.len() != 128 {
            fail("--user qt1: tem de ter 128 caracteres hex depois de qt1:");
        }
        u = rest[64..].to_string();
    }
    if !qt_server::protocol::is_valid_user_id(&u) {
        fail("--user tem de ser o id do relay (64 caracteres hex), como aparece na denuncia");
    }
    u
}

fn when(ms: u64) -> String {
    // Data UTC simples (sem dependencias): AAAA-MM-DD HH:MM.
    let secs = ms / 1000;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", rem / 3600, rem % 3600 / 60)
}

async fn all_reports(c: &mut RelayClient) -> Vec<StoredReport> {
    let mut out: Vec<StoredReport> = Vec::new();
    loop {
        let after = out.last().map(|r| r.id).unwrap_or(0);
        match ask(c, ClientMessage::AdminListReports { after }).await {
            ServerMessage::Reports { reports, more } => {
                let got = reports.len();
                out.extend(reports);
                if !more || got == 0 {
                    return out;
                }
            }
            other => fail(&format!("resposta inesperada: {other:?}")),
        }
    }
}

async fn cmd_listar() {
    let seed = load_seed();
    let mut c = connect(&seed).await;
    let reports = all_reports(&mut c).await;
    let opened: Vec<(StoredReport, Result<serde_json::Value, String>)> = reports
        .into_iter()
        .map(|r| {
            let body = report::open(&seed, &r.sealed)
                .map_err(|_| "nao abre com esta chave".to_string())
                .and_then(|b| serde_json::from_slice(&b).map_err(|_| "conteudo invalido".to_string()));
            (r, body)
        })
        .collect();
    if flag("--json") {
        let list: Vec<serde_json::Value> = opened
            .iter()
            .map(|(r, b)| match b {
                Ok(v) => serde_json::json!({"id": r.id, "recebida_em_ms": r.received_at_ms, "denuncia": v}),
                Err(e) => serde_json::json!({"id": r.id, "recebida_em_ms": r.received_at_ms, "erro": e}),
            })
            .collect();
        println!("{}", serde_json::Value::Array(list));
        return;
    }
    if opened.is_empty() {
        println!("Sem denuncias.");
        return;
    }
    for (r, b) in &opened {
        println!("== Denuncia {} (recebida {})", r.id, when(r.received_at_ms));
        match b {
            Err(e) => println!("   {e}"),
            Ok(v) => {
                println!("   Denunciado: {}", v["denunciado_relay_id"].as_str().unwrap_or("?"));
                println!("   ID publico: {}", v["denunciado_public_id"].as_str().unwrap_or("?"));
                println!("   Motivo:     {}", v["motivo"].as_str().unwrap_or("?"));
                for m in v["mensagens"].as_array().into_iter().flatten() {
                    let who = if m["de_denunciado"].as_bool() == Some(true) { "ELE " } else { "quem denunciou" };
                    let at = m["enviada_em_ms"].as_u64().map(when).unwrap_or_default();
                    println!("   [{at}] {who}: {}", m["texto"].as_str().unwrap_or(""));
                }
            }
        }
        println!();
    }
    println!("Banir: --example denuncias -- banir --key ... --user <Denunciado>");
}

#[tokio::main]
async fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    match cmd.as_str() {
        "chave" => cmd_chave(),
        "listar" => cmd_listar().await,
        "apagar" => {
            let id: u64 = arg("--id").and_then(|v| v.parse().ok()).unwrap_or_else(|| fail("falta --id <numero>"));
            let seed = load_seed();
            let mut c = connect(&seed).await;
            ask(&mut c, ClientMessage::AdminDeleteReport { id }).await;
            println!("Denuncia {id} apagada.");
        }
        "banir" => {
            let user = user_arg();
            let seed = load_seed();
            let mut c = connect(&seed).await;
            ask(&mut c, ClientMessage::AdminBan { user_id: user.clone() }).await;
            println!("Conta {user} banida.");
        }
        "desbanir" => {
            let user = user_arg();
            let seed = load_seed();
            let mut c = connect(&seed).await;
            ask(&mut c, ClientMessage::AdminUnban { user_id: user.clone() }).await;
            println!("Conta {user} desbanida.");
        }
        "premium-dar" | "premium-tirar" => {
            let user = user_arg();
            let until_ms = if cmd == "premium-tirar" {
                0
            } else if flag("--sempre") {
                253_402_300_799_000 // 31/12/9999
            } else {
                let dias: u64 = arg("--dias").and_then(|v| v.parse().ok()).filter(|d| *d > 0)
                    .unwrap_or_else(|| fail("falta --dias <numero> ou --sempre"));
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
                now + dias * 86_400_000
            };
            let seed = load_seed();
            let mut c = connect(&seed).await;
            ask(&mut c, ClientMessage::AdminGrantPremium { user_id: user.clone(), until_ms }).await;
            if until_ms == 0 {
                println!("Premium tirado a {user}.");
            } else {
                println!("Premium dado a {user} ate {}.", when(until_ms));
            }
        }
        "premium-lista" => {
            let seed = load_seed();
            let mut c = connect(&seed).await;
            match ask(&mut c, ClientMessage::AdminListPremium).await {
                ServerMessage::PremiumList { entries } if flag("--json") => {
                    let v: Vec<serde_json::Value> = entries.iter().map(|e| serde_json::json!({"user_id": e.user_id, "until_ms": e.until_ms})).collect();
                    println!("{}", serde_json::Value::Array(v))
                }
                ServerMessage::PremiumList { entries } if entries.is_empty() => println!("Ninguem com premium."),
                ServerMessage::PremiumList { entries } => entries.iter().for_each(|e| println!("{}  ate {}", e.user_id, when(e.until_ms))),
                other => fail(&format!("resposta inesperada: {other:?}")),
            }
        }
        "premium-bloqueio" => {
            let on = match (flag("--ligar"), flag("--desligar")) {
                (true, true) => fail("escolhe --ligar ou --desligar"),
                (true, false) => Some(true),
                (false, true) => Some(false),
                (false, false) => None,
            };
            let seed = load_seed();
            let mut c = connect(&seed).await;
            match ask(&mut c, ClientMessage::AdminPremiumEnforce { on }).await {
                ServerMessage::PremiumEnforce { on } if flag("--json") => println!("{}", serde_json::json!({ "ligado": on })),
                ServerMessage::PremiumEnforce { on: true } => println!("Bloqueio premium LIGADO."),
                ServerMessage::PremiumEnforce { on: false } => println!("Bloqueio premium desligado."),
                other => fail(&format!("resposta inesperada: {other:?}")),
            }
        }
        "banidos" => {
            let seed = load_seed();
            let mut c = connect(&seed).await;
            match ask(&mut c, ClientMessage::AdminListBans).await {
                ServerMessage::Bans { user_ids } if flag("--json") => {
                    println!("{}", serde_json::json!(user_ids))
                }
                ServerMessage::Bans { user_ids } if user_ids.is_empty() => println!("Nenhuma conta banida."),
                ServerMessage::Bans { user_ids } => user_ids.iter().for_each(|u| println!("{u}")),
                other => fail(&format!("resposta inesperada: {other:?}")),
            }
        }
        _ => {
            eprintln!("Uso: --example denuncias -- (chave|listar|apagar|banir|desbanir|banidos|premium-dar|premium-tirar|premium-lista|premium-bloqueio) --key <ficheiro>");
            std::process::exit(2);
        }
    }
}
