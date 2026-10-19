//! Notificacoes com a app fechada, pelo Firebase Cloud Messaging (FCM) no Android e
//! direto pela Apple (APNs, `apns.rs`) no iPhone. Os limites de frequencia sao os mesmos.
//!
//! O relay so toca a campainha: quando chega uma mensagem para alguem que nao tem
//! nenhuma ligacao a espera (`WaitMessages`, ou seja, a app esta fechada ou em
//! segundo plano ha mais de 30 s), pede a Google para acordar o telemovel dessa
//! pessoa. A mensagem para a Google e VAZIA: nao leva o remetente, o conteudo, o
//! tamanho, nem sequer o `user_id`. A Google fica a saber apenas que "algo" chegou
//! para aquele telemovel, e quando. O telemovel acorda, a app liga-se ao relay e
//! mostra "Tens mensagens novas", sem decifrar nada em segundo plano.
//!
//! Configuracao: `QT_FCM_CREDENTIALS` = caminho do JSON da conta de servico do
//! projeto Firebase (Definicoes do projeto > Contas de servico > Gerar chave
//! privada). Sem essa variavel, as notificacoes ficam desligadas e o resto do
//! relay funciona igual.
//!
//! Limites: no maximo uma campainha por utilizador a cada `MIN_INTERVAL` (a app,
//! acordada, vai buscar tudo o que estiver na fila). Um token que a Google diga
//! que ja nao existe (app desinstalada) e apagado do Redis.
//!
//! Chamadas de voz (desde a app 0.9.6, aprovado pelo Nuno a 19/10/2026): a oferta e o
//! cancelamento antes de atender vem marcados (`WakeKind`) e a campainha leva essa marca
//! (`"qt": "call"` / `"call_end"`), com prazo de 60 s. Assim uma app com o bloqueio ativo,
//! que nao consegue decifrar nada, sabe que deve tocar ou parar. A Google fica a saber o
//! momento de uma chamada (nao quem liga nem nada da chamada). Limites proprios: uma
//! campainha de chamada por utilizador a cada `MIN_INTERVAL`, e o "parar" so depois de
//! uma campainha de chamada ha menos de `CALL_END_WINDOW` (uma vez por chamada).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::protocol::WakeKind;
use crate::store::Store;

/// Intervalo minimo entre duas campainhas para o mesmo utilizador.
pub const MIN_INTERVAL: Duration = Duration::from_secs(10);
/// O "parar de tocar" so vale ate este tempo depois da campainha da chamada.
pub const CALL_END_WINDOW: Duration = Duration::from_secs(60);
const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";

#[derive(serde::Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    private_key: String,
    #[serde(default = "default_token_uri")]
    token_uri: String,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".to_string()
}

struct Pusher {
    sa: ServiceAccount,
    key: jsonwebtoken::EncodingKey,
    http: reqwest::Client,
    /// Endereco do FCM. So os testes o mudam (`QT_FCM_BASE_URL`, servidor falso local).
    fcm_base: String,
    /// Token de acesso OAuth da Google (valido 1 h; renovado aos 50 min).
    access: tokio::sync::Mutex<Option<(String, Instant)>>,
}

/// Limites de frequencia, iguais para o Android (FCM) e o iPhone (APNs, ver `apns.rs`).
#[derive(Default)]
struct Limits {
    /// Ultima campainha por utilizador (limite de frequencia).
    last: Mutex<HashMap<String, Instant>>,
    /// Ultima campainha de chamada por utilizador (limite proprio e "parar de tocar").
    calls: Mutex<HashMap<String, Instant>>,
}

static LIMITS: OnceLock<Limits> = OnceLock::new();

fn limits() -> &'static Limits {
    LIMITS.get_or_init(Limits::default)
}

static PUSHER: OnceLock<Option<Pusher>> = OnceLock::new();

fn pusher() -> Option<&'static Pusher> {
    PUSHER.get().and_then(|p| p.as_ref())
}

/// Le `QT_FCM_CREDENTIALS` (chamar uma vez, no arranque). Devolve se ficou ativo.
pub fn init_from_env() -> Result<bool, String> {
    let Some(path) = std::env::var("QT_FCM_CREDENTIALS").ok().filter(|p| !p.is_empty()) else {
        let _ = PUSHER.set(None);
        return Ok(false);
    };
    let json = std::fs::read_to_string(&path).map_err(|_| "nao foi possivel ler QT_FCM_CREDENTIALS".to_string())?;
    let p = from_json(&json)?;
    let _ = PUSHER.set(Some(p));
    Ok(true)
}

fn from_json(json: &str) -> Result<Pusher, String> {
    let sa: ServiceAccount = serde_json::from_str(json).map_err(|_| "JSON da conta de servico invalido".to_string())?;
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(sa.private_key.as_bytes())
        .map_err(|_| "chave privada da conta de servico invalida".to_string())?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "cliente HTTPS".to_string())?;
    let fcm_base = std::env::var("QT_FCM_BASE_URL").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| "https://fcm.googleapis.com".to_string());
    Ok(Pusher { sa, key, http, fcm_base, access: tokio::sync::Mutex::new(None) })
}

/// Ha alguma campainha ligada (Google para o Android ou Apple para o iPhone)?
pub fn enabled() -> bool {
    pusher().is_some() || crate::apns::enabled()
}

/// Chegou uma mensagem para `user_id`. Se as notificacoes estiverem ativas, ninguem
/// desse utilizador estiver a espera e nao tiver havido campainha ha menos de
/// `MIN_INTERVAL`, toca (em segundo plano: nao atrasa a resposta a quem enviou).
pub async fn after_enqueue(store: &Store, user_id: &str, wake: Option<WakeKind>) {
    if !enabled() {
        return;
    }
    if crate::waiters::is_waiting(user_id) {
        return;
    }
    let l = limits();
    let allowed = match wake {
        None => l.allow(user_id),
        Some(k) => l.allow_call(user_id, k),
    };
    if !allowed {
        return;
    }
    // A ligacao ao Redis ja existe (partilhada): isto e imediato.
    let Ok(mut conn) = store.connection().await else { return };
    let key = format!("{}:push:{}", store.key_prefix, user_id);
    tokio::spawn(async move {
        let token: Option<String> = redis::AsyncCommands::get(&mut conn, &key).await.unwrap_or(None);
        let Some(token) = token else { return };
        // iPhone (APNs, direto pela Apple) ou Android (FCM).
        let result = if token.starts_with(crate::apns::TOKEN_PREFIX) {
            match crate::apns::send(&token, wake).await {
                Ok(true) => Ok(()),
                Ok(false) => return,   // nada a enviar (ex.: "parar de tocar" ao iPhone)
                Err(crate::apns::SendError::Unregistered) => Err(SendError::Unregistered),
                Err(crate::apns::SendError::Other) => Err(SendError::Other),
            }
        } else {
            let Some(p) = pusher() else { return };
            p.send(&token, wake).await
        };
        match result {
            Ok(()) => crate::stats::inc(&crate::stats::STATS.pushes),
            Err(SendError::Unregistered) => {
                crate::stats::inc(&crate::stats::STATS.push_failed);
                let _: Result<(), _> = redis::AsyncCommands::del(&mut conn, &key).await;
            }
            Err(SendError::Other) => crate::stats::inc(&crate::stats::STATS.push_failed),
        }
    });
}

enum SendError {
    /// A Google diz que este token ja nao existe (app desinstalada ou dados apagados).
    Unregistered,
    Other,
}

impl Limits {
    fn allow(&self, user_id: &str) -> bool {
        let now = Instant::now();
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if last.get(user_id).is_some_and(|t| now.duration_since(*t) < MIN_INTERVAL) {
            return false;
        }
        // Nao deixa o mapa crescer para sempre: tira as entradas antigas.
        if last.len() > 10_000 {
            last.retain(|_, t| now.duration_since(*t) < MIN_INTERVAL);
        }
        last.insert(user_id.to_string(), now);
        true
    }

    /// Chamadas: uma campainha a cada `MIN_INTERVAL`; o "parar" so com uma campainha de
    /// chamada ha menos de `CALL_END_WINDOW`, e so uma vez.
    fn allow_call(&self, user_id: &str, kind: WakeKind) -> bool {
        let now = Instant::now();
        let mut calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
        if calls.len() > 10_000 {
            calls.retain(|_, t| now.duration_since(*t) < CALL_END_WINDOW);
        }
        let recent = calls.get(user_id).map(|t| now.duration_since(*t));
        match kind {
            WakeKind::Call => {
                if recent.is_some_and(|d| d < MIN_INTERVAL) {
                    return false;
                }
                calls.insert(user_id.to_string(), now);
                true
            }
            WakeKind::CallEnd => {
                if recent.is_some_and(|d| d < CALL_END_WINDOW) {
                    calls.remove(user_id);
                    true
                } else {
                    false
                }
            }
        }
    }
}

impl Pusher {
    async fn access_token(&self) -> Option<String> {
        let mut cached = self.access.lock().await;
        if let Some((t, at)) = cached.as_ref() {
            if at.elapsed() < Duration::from_secs(50 * 60) {
                return Some(t.clone());
            }
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        let claims = serde_json::json!({
            "iss": self.sa.client_email,
            "scope": SCOPE,
            "aud": self.sa.token_uri,
            "iat": now,
            "exp": now + 3600,
        });
        let jwt = jsonwebtoken::encode(&jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256), &claims, &self.key).ok()?;
        let resp = self
            .http
            .post(&self.sa.token_uri)
            .form(&[("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"), ("assertion", jwt.as_str())])
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            eprintln!("notificacoes: a Google recusou a conta de servico (HTTP {})", resp.status().as_u16());
            return None;
        }
        let body: serde_json::Value = resp.json().await.ok()?;
        let token = body.get("access_token")?.as_str()?.to_string();
        *cached = Some((token.clone(), Instant::now()));
        Some(token)
    }

    async fn send(&self, device_token: &str, wake: Option<WakeKind>) -> Result<(), SendError> {
        let access = self.access_token().await.ok_or(SendError::Other)?;
        let url = format!("{}/v1/projects/{}/messages:send", self.fcm_base, self.sa.project_id);
        let resp = self
            .http
            .post(url)
            .bearer_auth(access)
            .json(&wake_message(device_token, wake))
            .send()
            .await
            .map_err(|_| SendError::Other)?;
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        if status == 404 || body.contains("UNREGISTERED") || (status == 400 && body.contains("registration token")) {
            Err(SendError::Unregistered)
        } else {
            Err(SendError::Other)
        }
    }
}

/// A mensagem que vai para a Google: so o token do telemovel, prioridade alta (para
/// acordar mesmo em poupanca de bateria) e um campo fixo: "1" para mensagens, "call" e
/// "call_end" para chamadas (com prazo de 60 s: uma chamada atrasada ja nao serve).
pub fn wake_message(device_token: &str, wake: Option<WakeKind>) -> serde_json::Value {
    let (qt, ttl, collapse) = match wake {
        None => ("1", "3600s", "qt"),
        Some(WakeKind::Call) => ("call", "60s", "qt-call"),
        Some(WakeKind::CallEnd) => ("call_end", "60s", "qt-call"),
    };
    serde_json::json!({
        "message": {
            "token": device_token,
            "android": { "priority": "HIGH", "ttl": ttl, "collapse_key": collapse },
            "data": { "qt": qt }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mensagem_para_a_google_nao_leva_nada_da_mensagem() {
        let m = wake_message("tok_123", None);
        let s = m.to_string();
        assert!(s.contains("tok_123"));
        assert_eq!(m["message"]["data"], serde_json::json!({ "qt": "1" }));
        assert!(m["message"].get("notification").is_none(), "sem texto visivel enviado pela Google");
        let c = wake_message("tok_123", Some(WakeKind::Call));
        assert_eq!(c["message"]["data"], serde_json::json!({ "qt": "call" }));
        assert_eq!(c["message"]["android"]["ttl"], "60s");
        assert!(c["message"].get("notification").is_none());
        let e = wake_message("tok_123", Some(WakeKind::CallEnd));
        assert_eq!(e["message"]["data"], serde_json::json!({ "qt": "call_end" }));
    }

    #[test]
    fn limites_das_chamadas() {
        let p = Limits::default();
        assert!(!p.allow_call("a", WakeKind::CallEnd), "parar sem chamada: nao");
        assert!(p.allow_call("a", WakeKind::Call));
        assert!(!p.allow_call("a", WakeKind::Call), "segunda chamada logo a seguir: nao");
        assert!(p.allow("a"), "o limite das mensagens e outro");
        assert!(p.allow_call("a", WakeKind::CallEnd), "parar a chamada que tocou");
        assert!(!p.allow_call("a", WakeKind::CallEnd), "so uma vez");
        assert!(p.allow_call("a", WakeKind::Call), "depois de parar pode voltar a ligar");
        assert!(p.allow_call("b", WakeKind::Call), "outro utilizador: sim");
    }

    #[test]
    fn limite_de_frequencia_por_utilizador() {
        let p = Limits::default();
        assert!(p.allow("a"));
        assert!(!p.allow("a"), "segunda campainha logo a seguir: nao");
        assert!(p.allow("b"), "outro utilizador: sim");
    }

    #[test]
    fn credenciais_invalidas_sao_recusadas() {
        assert!(from_json("{}").is_err());
        assert!(from_json(r#"{"project_id":"p","client_email":"e","private_key":"nao e uma chave"}"#).is_err());
    }
}
