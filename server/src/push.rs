//! Notificacoes com a app fechada, pelo Firebase Cloud Messaging (FCM).
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

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::store::Store;

/// Intervalo minimo entre duas campainhas para o mesmo utilizador.
pub const MIN_INTERVAL: Duration = Duration::from_secs(10);
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
    /// Token de acesso OAuth da Google (valido 1 h; renovado aos 50 min).
    access: tokio::sync::Mutex<Option<(String, Instant)>>,
    /// Ultima campainha por utilizador (limite de frequencia).
    last: Mutex<HashMap<String, Instant>>,
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
    Ok(Pusher { sa, key, http, access: tokio::sync::Mutex::new(None), last: Mutex::new(HashMap::new()) })
}

pub fn enabled() -> bool {
    pusher().is_some()
}

/// Chegou uma mensagem para `user_id`. Se as notificacoes estiverem ativas, ninguem
/// desse utilizador estiver a espera e nao tiver havido campainha ha menos de
/// `MIN_INTERVAL`, toca (em segundo plano: nao atrasa a resposta a quem enviou).
pub async fn after_enqueue(store: &Store, user_id: &str) {
    let Some(p) = pusher() else { return };
    if crate::waiters::is_waiting(user_id) || !p.allow(user_id) {
        return;
    }
    // A ligacao ao Redis ja existe (partilhada): isto e imediato.
    let Ok(mut conn) = store.connection().await else { return };
    let key = format!("{}:push:{}", store.key_prefix, user_id);
    tokio::spawn(async move {
        let token: Option<String> = redis::AsyncCommands::get(&mut conn, &key).await.unwrap_or(None);
        let Some(token) = token else { return };
        match p.send(&token).await {
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

impl Pusher {
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

    async fn send(&self, device_token: &str) -> Result<(), SendError> {
        let access = self.access_token().await.ok_or(SendError::Other)?;
        let url = format!("https://fcm.googleapis.com/v1/projects/{}/messages:send", self.sa.project_id);
        let resp = self
            .http
            .post(url)
            .bearer_auth(access)
            .json(&wake_message(device_token))
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
/// acordar mesmo em poupanca de bateria) e um campo fixo, igual para todos.
pub fn wake_message(device_token: &str) -> serde_json::Value {
    serde_json::json!({
        "message": {
            "token": device_token,
            "android": { "priority": "HIGH", "ttl": "3600s", "collapse_key": "qt" },
            "data": { "qt": "1" }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mensagem_para_a_google_nao_leva_nada_da_mensagem() {
        let m = wake_message("tok_123");
        let s = m.to_string();
        assert!(s.contains("tok_123"));
        assert_eq!(m["message"]["data"], serde_json::json!({ "qt": "1" }));
        assert!(m["message"].get("notification").is_none(), "sem texto visivel enviado pela Google");
    }

    #[test]
    fn limite_de_frequencia_por_utilizador() {
        let p = Pusher {
            sa: ServiceAccount { project_id: "p".into(), client_email: "e".into(), private_key: String::new(), token_uri: default_token_uri() },
            key: jsonwebtoken::EncodingKey::from_secret(b"x"),
            http: reqwest::Client::new(),
            access: tokio::sync::Mutex::new(None),
            last: Mutex::new(HashMap::new()),
        };
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
