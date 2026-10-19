//! Notificacoes para o iPhone, direto pela Apple (APNs), sem Firebase.
//!
//! Igual ao FCM (ver `push.rs`), o relay so toca a campainha. A app do iPhone manda ao
//! relay um token no formato `apns:<ambiente>:<token de alertas>:<token VoIP>`:
//!
//!  - `ambiente`: `p` (App Store / TestFlight) ou `s` (sandbox, apps instaladas pelo Xcode).
//!  - mensagens: um alerta com o texto FIXO da propria app (`loc-key`, traduzido no
//!    iPhone), "Tens mensagens novas". A Apple nao recebe o remetente, o conteudo nem o
//!    `user_id`; so que "algo" chegou para aquele iPhone, e quando.
//!  - chamadas (`WakeKind::Call`): um push VoIP (PushKit) vazio, `{"qt":"call"}`, com prazo
//!    de 60 s. O iOS acorda a app, que mostra a chamada pelo CallKit. Como no Android
//!    (aprovado pelo Nuno a 19/10/2026), a Apple fica a saber o momento de uma chamada.
//!  - `WakeKind::CallEnd` nao vai para o iPhone: o iOS obriga cada push VoIP a mostrar uma
//!    chamada nova. A app aberta pela chamada fica a espera no relay e ve o "desligou" la.
//!
//! Configuracao: `QT_APNS_CREDENTIALS` = caminho de um JSON com
//! `{"team_id": "...", "key_id": "...", "topic": "sealer.app", "private_key": "-----BEGIN PRIVATE KEY-----..."}`
//! (a chave `.p8` criada em developer.apple.com > Keys, com "Apple Push Notifications
//! service"). Sem essa variavel, o iPhone fica sem notificacoes e o resto funciona igual.

use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::protocol::WakeKind;

/// Prefixo dos tokens do iPhone (os do Android, do FCM, nunca comecam assim).
pub const TOKEN_PREFIX: &str = "apns:";
/// Tamanho de um token da Apple: 32 bytes em hex (pode crescer; aceita-se ate 200).
const MAX_DEVICE_TOKEN_HEX: usize = 200;
/// A Apple pede um JWT novo entre 20 e 60 minutos.
const JWT_REFRESH: Duration = Duration::from_secs(40 * 60);

#[derive(serde::Deserialize)]
struct Credentials {
    team_id: String,
    key_id: String,
    topic: String,
    private_key: String,
}

struct Apns {
    team_id: String,
    key_id: String,
    topic: String,
    key: jsonwebtoken::EncodingKey,
    http: reqwest::Client,
    /// So os testes mudam (`QT_APNS_BASE_URL`).
    base_override: Option<String>,
    jwt: std::sync::Mutex<Option<(String, Instant)>>,
}

static APNS: OnceLock<Option<Apns>> = OnceLock::new();

fn apns() -> Option<&'static Apns> {
    APNS.get().and_then(|a| a.as_ref())
}

pub fn enabled() -> bool {
    apns().is_some()
}

/// Le `QT_APNS_CREDENTIALS` (chamar uma vez, no arranque). Devolve se ficou ativo.
pub fn init_from_env() -> Result<bool, String> {
    let Some(path) = std::env::var("QT_APNS_CREDENTIALS").ok().filter(|p| !p.is_empty()) else {
        let _ = APNS.set(None);
        return Ok(false);
    };
    let json = std::fs::read_to_string(&path).map_err(|_| "nao foi possivel ler QT_APNS_CREDENTIALS".to_string())?;
    let a = from_json(&json)?;
    let _ = APNS.set(Some(a));
    Ok(true)
}

fn from_json(json: &str) -> Result<Apns, String> {
    let c: Credentials = serde_json::from_str(json).map_err(|_| "JSON das credenciais da Apple invalido".to_string())?;
    let simple = |s: &str| !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b));
    if !simple(&c.team_id) || !simple(&c.key_id) || !simple(&c.topic) {
        return Err("team_id, key_id ou topic invalidos".into());
    }
    let key = jsonwebtoken::EncodingKey::from_ec_pem(c.private_key.as_bytes())
        .map_err(|_| "chave privada da Apple (.p8) invalida".to_string())?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "cliente HTTPS".to_string())?;
    let base_override = std::env::var("QT_APNS_BASE_URL").ok().filter(|u| !u.is_empty());
    Ok(Apns {
        team_id: c.team_id,
        key_id: c.key_id,
        topic: c.topic,
        key,
        http,
        base_override,
        jwt: std::sync::Mutex::new(None),
    })
}

/// Token do iPhone separado nas suas partes.
#[derive(Debug, PartialEq, Eq)]
pub struct DeviceToken<'a> {
    pub sandbox: bool,
    pub alert: &'a str,
    pub voip: &'a str,
}

fn is_hex_token(t: &str) -> bool {
    t.len() <= MAX_DEVICE_TOKEN_HEX && t.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `apns:p:<alertas>:<voip>` (qualquer um dos dois pode estar vazio, nao os dois).
pub fn parse_token(token: &str) -> Option<DeviceToken<'_>> {
    let rest = token.strip_prefix(TOKEN_PREFIX)?;
    let mut parts = rest.split(':');
    let env = parts.next()?;
    let alert = parts.next()?;
    let voip = parts.next().unwrap_or("");
    if parts.next().is_some() {
        return None;
    }
    let sandbox = match env {
        "p" => false,
        "s" => true,
        _ => return None,
    };
    if !is_hex_token(alert) || !is_hex_token(voip) || (alert.is_empty() && voip.is_empty()) {
        return None;
    }
    Some(DeviceToken { sandbox, alert, voip })
}

/// O que se envia a Apple: o token a usar, o tipo, a prioridade, o prazo e o corpo.
#[derive(Debug, PartialEq)]
pub struct Notice {
    pub device: String,
    pub push_type: &'static str,
    pub topic_suffix: &'static str,
    pub expiration_secs: u64,
    pub collapse_id: Option<&'static str>,
    pub body: serde_json::Value,
}

/// Monta a campainha. `None`: nada a enviar (ex.: "parar de tocar" para o iPhone).
pub fn notice(t: &DeviceToken<'_>, wake: Option<WakeKind>) -> Option<Notice> {
    match wake {
        None => (!t.alert.is_empty()).then(|| Notice {
            device: t.alert.to_ascii_lowercase(),
            push_type: "alert",
            topic_suffix: "",
            expiration_secs: 3600,
            collapse_id: Some("qt"),
            // Texto fixo da propria app (Localizable.strings): a Apple nao ve nada da mensagem.
            body: serde_json::json!({
                "aps": { "alert": { "title": "SEALER", "loc-key": "push_new" }, "sound": "default", "thread-id": "qt" }
            }),
        }),
        Some(WakeKind::Call) => (!t.voip.is_empty()).then(|| Notice {
            device: t.voip.to_ascii_lowercase(),
            push_type: "voip",
            topic_suffix: ".voip",
            expiration_secs: 60,
            collapse_id: None,
            body: serde_json::json!({ "qt": "call" }),
        }),
        Some(WakeKind::CallEnd) => None,
    }
}

pub enum SendError {
    /// A Apple diz que este token ja nao serve (app apagada, notificacoes desligadas).
    Unregistered,
    Other,
}

impl Apns {
    fn jwt(&self) -> Option<String> {
        let mut cached = self.jwt.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((t, at)) = cached.as_ref() {
            if at.elapsed() < JWT_REFRESH {
                return Some(t.clone());
            }
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some(self.key_id.clone());
        let claims = serde_json::json!({ "iss": self.team_id, "iat": now });
        let t = jsonwebtoken::encode(&header, &claims, &self.key).ok()?;
        *cached = Some((t.clone(), Instant::now()));
        Some(t)
    }

    async fn send(&self, n: &Notice, sandbox: bool) -> Result<(), SendError> {
        let jwt = self.jwt().ok_or(SendError::Other)?;
        let base = match &self.base_override {
            Some(b) => b.clone(),
            None if sandbox => "https://api.sandbox.push.apple.com".to_string(),
            None => "https://api.push.apple.com".to_string(),
        };
        let exp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) + n.expiration_secs;
        let mut req = self
            .http
            .post(format!("{base}/3/device/{}", n.device))
            .header("authorization", format!("bearer {jwt}"))
            .header("apns-topic", format!("{}{}", self.topic, n.topic_suffix))
            .header("apns-push-type", n.push_type)
            .header("apns-priority", "10")
            .header("apns-expiration", exp.to_string())
            .json(&n.body);
        if let Some(c) = n.collapse_id {
            req = req.header("apns-collapse-id", c);
        }
        let resp = req.send().await.map_err(|_| SendError::Other)?;
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        if status == 410 || body.contains("BadDeviceToken") || body.contains("Unregistered") || body.contains("DeviceTokenNotForTopic") {
            Err(SendError::Unregistered)
        } else {
            if body.contains("InvalidProviderToken") || body.contains("ExpiredProviderToken") {
                // JWT recusado: gera outro na proxima.
                *self.jwt.lock().unwrap_or_else(|e| e.into_inner()) = None;
                eprintln!("notificacoes (Apple): credenciais recusadas (HTTP {status})");
            }
            Err(SendError::Other)
        }
    }
}

/// Envia a campainha para um token `apns:...`. `Ok(false)`: nada a enviar.
pub async fn send(token: &str, wake: Option<WakeKind>) -> Result<bool, SendError> {
    let Some(a) = apns() else { return Ok(false) };
    let Some(t) = parse_token(token) else { return Err(SendError::Unregistered) };
    let Some(n) = notice(&t, wake) else { return Ok(false) };
    a.send(&n, t.sandbox).await.map(|_| true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const V: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    #[test]
    fn formato_do_token() {
        let full = format!("apns:p:{A}:{V}");
        let t = parse_token(&full).unwrap();
        assert_eq!(t, DeviceToken { sandbox: false, alert: A, voip: V });
        let sb = format!("apns:s:{A}:");
        let s = parse_token(&sb).unwrap();
        assert!(s.sandbox && s.voip.is_empty());
        assert!(parse_token(&format!("apns:s::{V}")).is_some(), "so VoIP");
        assert!(parse_token("apns:p::").is_none(), "nenhum token");
        assert!(parse_token(&format!("apns:x:{A}:")).is_none(), "ambiente desconhecido");
        assert!(parse_token(&format!("apns:p:{A}:{V}:mais")).is_none());
        assert!(parse_token("apns:p:nao-hex:").is_none());
        assert!(parse_token(&format!("fcm:{A}")).is_none());
        assert!(crate::protocol::is_valid_push_token(&format!("apns:p:{A}:{V}")), "o relay aceita o formato");
    }

    #[test]
    fn a_campainha_nao_leva_nada_da_mensagem() {
        let full = format!("apns:p:{A}:{V}");
        let t = parse_token(&full).unwrap();
        let m = notice(&t, None).unwrap();
        assert_eq!(m.device, A);
        assert_eq!(m.push_type, "alert");
        assert_eq!(m.body["aps"]["alert"]["loc-key"], "push_new");
        assert!(m.body.to_string().len() < 200, "corpo curto e fixo");
        let c = notice(&t, Some(WakeKind::Call)).unwrap();
        assert_eq!(c.device, V);
        assert_eq!((c.push_type, c.topic_suffix, c.expiration_secs), ("voip", ".voip", 60));
        assert_eq!(c.body, serde_json::json!({ "qt": "call" }));
        assert!(notice(&t, Some(WakeKind::CallEnd)).is_none(), "o iOS nao deixa um push VoIP sem chamada");
        let so = format!("apns:p:{A}:");
        let so_alertas = parse_token(&so).unwrap();
        assert!(notice(&so_alertas, Some(WakeKind::Call)).is_none());
    }

    /// Chave P-256 so para este teste (gerada a 19/10/2026, nao serve para nada real).
    const TEST_KEY: &str = "-----BEGIN PRIVATE KEY-----\\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgWbhPdXkDyc14E1os\\n8rd5w/wSGXjXAxVLgkeTaVscFFChRANCAAS28NGWP75dvcqJOOlaDsUYw1DmBYrq\\ndwE489nfwmUddli0evtbWkSExn5BH7YMXJt0eytx+VKhdGKkigOjvteD\\n-----END PRIVATE KEY-----\\n";

    #[test]
    fn jwt_es256_com_a_chave_da_apple() {
        let json = format!(r#"{{"team_id":"TEAM123456","key_id":"KEY1234567","topic":"sealer.app","private_key":"{TEST_KEY}"}}"#);
        let a = from_json(&json).expect("credenciais validas");
        let t = a.jwt().expect("jwt");
        let header = jsonwebtoken::decode_header(&t).unwrap();
        assert_eq!(header.alg, jsonwebtoken::Algorithm::ES256);
        assert_eq!(header.kid.as_deref(), Some("KEY1234567"));
        assert_eq!(a.jwt().unwrap(), t, "reutilizado ate aos 40 min");
    }

    #[test]
    fn credenciais_invalidas_sao_recusadas() {
        assert!(from_json("{}").is_err());
        assert!(from_json(r#"{"team_id":"T","key_id":"K","topic":"sealer.app","private_key":"nao e uma chave"}"#).is_err());
        assert!(from_json(r#"{"team_id":"T/../","key_id":"K","topic":"sealer.app","private_key":"x"}"#).is_err());
    }
}
