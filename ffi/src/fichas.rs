//! Fichas anonimas do video pelo relay (patch 0006), do lado da app.
//!
//! A app pede as fichas do mes num lote, CEGAS (`qt_core::fichas`), ao relay das mensagens
//! (a conta autenticada so serve para o relay contar quantas deu a esta conta este mes). A
//! prova DLEQ da resposta e verificada aqui; as fichas ficam na base cifrada (SQLCipher,
//! tabela `fichas`), apagadas quando se usam e quando o mes acaba.
//!
//! QUANDO PEDIR (decisao do 0006): nunca no inicio de uma chamada (o relay podia juntar a hora
//! do pedido a hora de uma chamada). A app pergunta `fichas_status().due` numa hora aleatoria
//! depois de abrir (e de hora a hora), fora das chamadas; `relay_request_fichas` recusa-se
//! (`None`) com uma chamada a decorrer.
//!
//! CHAVE PUBLICADA (obrigatoria): a app vai buscar ao site (`https://sealer.pt/estado.json`, fora
//! do relay, publicado no deploy) as chaves das fichas de cada mes e entrega-as aqui
//! (`fichas_set_published_keys`). Sem a chave deste mes, ou com uma resposta do emissor com outra
//! chave, nada se pede nem se guarda; na sala, o motor so gasta se a chave do relay for a
//! publicada. Um relay comprometido que desse uma chave diferente a cada conta (particao de
//! chaves, para reconhecer quem gasta) nao passa daqui. Site indisponivel: falha fechada (sem
//! video pelo relay; voz e ligacao direta normais).
//!
//! MES DO RELAY: se o relay responder com outro mes (relogio deste telemovel errado), a app so o
//! segue se for o vizinho e estivermos a menos de 48 h da viragem (`epoch_plausible`); e o motor
//! so recebe as chaves publicadas desses meses. Um relay comprometido que dissesse a cada conta
//! um mes diferente (o site publica 24) para as separar em grupos pelas fichas que gastam (uma
//! particao como a das chaves, mas pelo mes) nao passa daqui.
//!
//! Para cada chamada, o motor recebe uma carteira em memoria com as fichas deste mes
//! (`wallet_for_call`); as que ele usar sao apagadas da base a seguir (`flush_used`). Se a app
//! morrer antes de as apagar, o relay recusa-as como ja gastas e o motor passa a seguinte.

use base64::Engine;
use qt_core::fichas::{current_epoch, epoch_plausible, unix_now, Ficha, IssueRequest, DEFAULT_PER_MONTH, MAX_BATCH};
use qt_storage as st;

use crate::client::{bad, QtClient, QtError};

/// Mes para o qual ja nao ha mais nada a pedir (o relay disse que nao ha mais fichas).
const KV_DONE: &str = "fichas:done";
/// Quantas o relay disse que ainda faltam este mes: `[4 epoca][4 quantas]`.
const KV_REMAINING: &str = "fichas:remaining";
/// Depois de o relay nao emitir (antigo, desligado, erro): so voltar a pedir depois disto.
const KV_RETRY_AT: &str = "fichas:retry_at";
/// Um pedido sem resposta ha mais disto ja nao conta (a app pode pedir outra vez).
const PENDING_TTL_SECS: u64 = 120;
/// Espera depois de um relay que nao emite: 6 h.
const RETRY_AFTER_SECS: u64 = 6 * 3600;
/// Chaves publicadas no site: `[4 epoca][32 chave]` por mes.
const KV_PUBLISHED: &str = "fichas:publicadas";
/// Meses aceites no ficheiro do site (o deploy publica 24).
const MAX_PUBLISHED: usize = 120;

/// Estado das fichas em memoria (por `QtClient`).
#[derive(Default)]
pub(crate) struct FichasState {
    /// Pedido enviado, a espera da resposta: os fatores de cegueira so existem aqui.
    pending: Option<(IssueRequest, u64)>,
    /// Mes do relay, quando difere do relogio deste telemovel (resposta com outra epoca).
    epoch_hint: Option<u32>,
    /// Carteira da chamada atual (o motor marca as usadas).
    #[cfg(feature = "calls")]
    wallet: Option<std::sync::Arc<qt_call::fichas::MemWallet>>,
}

/// O que a app precisa de saber.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FichasStatus {
    /// Mes (AAAAMM, UTC).
    pub epoch: u32,
    /// Fichas guardadas para este mes (1 ficha = 1 minuto de video pelo relay).
    pub left: u32,
    /// Vale a pena pedir agora (`relay_request_fichas`): faltam fichas deste mes, nao ha chamada
    /// e a chave deste mes publicada no site ja chegou.
    pub due: bool,
    /// A chave deste mes publicada no site esta guardada (sem ela: falha fechada).
    pub published: bool,
}

/// Resposta `Fichas` do relay (`relay_parse_response`), para `fichas_accept`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FichasIssued {
    pub epoch: u32,
    pub public_key: Vec<u8>,
    pub evaluated: Vec<u8>,
    pub proof: Vec<u8>,
    pub granted: u32,
    pub remaining: u32,
    pub per_month: u32,
}

fn kv_u32s(c: &st::rusqlite::Connection, key: &str) -> Result<Vec<u32>, QtError> {
    Ok(st::kv_get(c, key)?
        .map(|v| v.chunks_exact(4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])).collect())
        .unwrap_or_default())
}

/// Chaves publicadas no site guardadas (`(AAAAMM, chave)`).
pub(crate) fn published_keys(c: &st::rusqlite::Connection) -> Result<Vec<(u32, [u8; 32])>, QtError> {
    Ok(st::kv_get(c, KV_PUBLISHED)?
        .map(|v| {
            v.chunks_exact(36)
                .filter_map(|b| Some((u32::from_be_bytes(b[..4].try_into().ok()?), b[4..].try_into().ok()?)))
                .collect()
        })
        .unwrap_or_default())
}

fn published_key(c: &st::rusqlite::Connection, epoch: u32) -> Result<Option<[u8; 32]>, QtError> {
    Ok(published_keys(c)?.into_iter().find(|(e, _)| *e == epoch).map(|(_, k)| k))
}

/// Le o ficheiro do site: `{"fichas": {"AAAAMM": "<64 hex>", ...}}` (o resto e ignorado).
fn parse_published(json: &[u8]) -> Option<Vec<(u32, [u8; 32])>> {
    let v: serde_json::Value = serde_json::from_slice(json).ok()?;
    let map = v.get("fichas")?.as_object()?;
    if map.is_empty() || map.len() > MAX_PUBLISHED {
        return None;
    }
    let mut out = Vec::with_capacity(map.len());
    for (k, val) in map {
        let epoch: u32 = k.parse().ok().filter(|_| k.len() == 6)?;
        if !qt_core::fichas::epoch_valid(epoch) {
            return None;
        }
        let hex = val.as_str()?;
        let bytes = crate::client::unhex(hex).filter(|_| hex.len() == 64)?;
        out.push((epoch, bytes.try_into().ok()?));
    }
    Some(out)
}

fn kv_u64(c: &st::rusqlite::Connection, key: &str) -> Result<u64, QtError> {
    Ok(st::kv_get(c, key)?.and_then(|v| v.try_into().ok()).map(u64::from_be_bytes).unwrap_or(0))
}

impl QtClient {
    /// Mes das fichas: o do relay, se ja disse que e outro e for plausivel (o vizinho, perto da
    /// viragem); senao o do relogio deste telemovel.
    fn fichas_epoch(&self) -> u32 {
        crate::client::relock(&self.fichas).epoch_hint.filter(|e| epoch_plausible(*e)).unwrap_or_else(current_epoch)
    }

    fn in_call(&self) -> bool {
        crate::client::relock(&self.calls).in_call()
    }
}

/// Apaga da base as fichas que o motor ja usou (chamar com a base aberta).
pub(crate) fn flush_used_with(client: &QtClient, c: &st::rusqlite::Connection) -> Result<(), QtError> {
    #[cfg(feature = "calls")]
    {
        let w = crate::client::relock(&client.fichas).wallet.clone();
        if let Some(w) = w {
            for n in w.take_used() {
                st::fichas_delete(c, &n)?;
            }
        }
    }
    #[cfg(not(feature = "calls"))]
    let _ = (client, c);
    Ok(())
}

/// Carteira de uma chamada e as chaves publicadas no site (`(epoca, chave)`).
#[cfg(feature = "calls")]
pub(crate) type CallWallet = (std::sync::Arc<qt_call::fichas::MemWallet>, Vec<(u32, [u8; 32])>);

/// Carteira para o motor da chamada que vai comecar: as fichas deste mes, ja sem as de meses
/// passados (apagadas aqui), e so as que tem a chave publicada no site.
#[cfg(feature = "calls")]
/// Devolve tambem as chaves publicadas no site (o motor so gasta se a chave da sala for uma delas).
pub(crate) fn wallet_for_call(client: &QtClient) -> Result<CallWallet, QtError> {
    let epoch = client.fichas_epoch();
    let c = client.lock()?;
    flush_used_with(client, &c)?;
    st::fichas_delete_other_epochs(&c, epoch)?;
    // So as chaves de meses plausiveis agora (este, ou o vizinho perto da viragem): o motor nao
    // aceita da sala a chave de um mes qualquer dos 24 publicados.
    let published: Vec<(u32, [u8; 32])> = published_keys(&c)?.into_iter().filter(|(e, _)| epoch_plausible(*e)).collect();
    // So as emitidas com a chave publicada no site para o mes delas.
    let fichas: Vec<Ficha> = st::fichas_load(&c, epoch)?
        .iter()
        .filter_map(|b| Ficha::from_bytes(b))
        .filter(|f| published.contains(&(f.epoch, f.issuer_key)))
        .collect();
    let w = qt_call::fichas::MemWallet::new(fichas);
    crate::client::relock(&client.fichas).wallet = Some(w.clone());
    Ok((w, published))
}

#[uniffi::export]
impl QtClient {
    /// Fichas deste mes e se vale a pena pedir mais agora.
    pub fn fichas_status(&self) -> Result<FichasStatus, QtError> {
        let epoch = self.fichas_epoch();
        let in_call = self.in_call();
        let pending_fresh = crate::client::relock(&self.fichas).pending.as_ref().is_some_and(|(_, at)| unix_now().saturating_sub(*at) < PENDING_TTL_SECS);
        let c = self.lock()?;
        flush_used_with(self, &c)?;
        st::fichas_delete_other_epochs(&c, epoch)?;
        let left = st::fichas_count(&c, epoch)?;
        let done = kv_u32s(&c, KV_DONE)?.first() == Some(&epoch);
        let retry_ok = unix_now() >= kv_u64(&c, KV_RETRY_AT)?;
        let published = published_key(&c, epoch)?.is_some();
        Ok(FichasStatus { epoch, left, due: !in_call && !done && retry_ok && !pending_fresh && published, published })
    }

    /// Chaves das fichas publicadas no site (`https://sealer.pt/estado.json`, os bytes do ficheiro
    /// tal como vieram). Substituem as guardadas. Devolve quantos meses tem. Ficheiro invalido:
    /// erro, e ficam as anteriores. A app chama isto antes de pedir fichas (e de hora a hora).
    pub fn fichas_set_published_keys(&self, json: Vec<u8>) -> Result<u32, QtError> {
        if json.len() > 64 * 1024 {
            return Err(bad("estado.json"));
        }
        let keys = parse_published(&json).ok_or_else(|| bad("estado.json"))?;
        let mut v = Vec::with_capacity(keys.len() * 36);
        for (e, k) in &keys {
            v.extend_from_slice(&e.to_be_bytes());
            v.extend_from_slice(k);
        }
        let c = self.lock()?;
        st::kv_set(&c, KV_PUBLISHED, &v)?;
        Ok(keys.len() as u32)
    }

    /// Pedido de fichas para o relay das mensagens (`IssueFichas`, cegas), ou `None` se nao for
    /// preciso agora (ja tem as do mes, o relay nao emite, ou ha uma chamada: nunca se pede no
    /// inicio de uma). Resposta: `RelayResponse::Fichas` -> `fichas_accept`; `Error` ->
    /// `fichas_refused`.
    pub fn relay_request_fichas(&self) -> Result<Option<Vec<u8>>, QtError> {
        let status = self.fichas_status()?;
        if !status.due {
            return Ok(None);
        }
        let n = {
            let c = self.lock()?;
            match kv_u32s(&c, KV_REMAINING)?.as_slice() {
                [e, left] if *e == status.epoch => *left as usize,
                _ => DEFAULT_PER_MONTH as usize,
            }
        }
        .min(MAX_BATCH);
        if n == 0 {
            let c = self.lock()?;
            st::kv_set(&c, KV_DONE, &status.epoch.to_be_bytes())?;
            return Ok(None);
        }
        let (req, blinded) = IssueRequest::new(status.epoch, n).map_err(|_| QtError::Crypto)?;
        crate::client::relock(&self.fichas).pending = Some((req, unix_now()));
        let body = serde_json::json!({"IssueFichas": {
            "epoch": status.epoch,
            "blinded": base64::engine::general_purpose::STANDARD.encode(&blinded),
        }});
        serde_json::to_vec(&body).map(Some).map_err(|_| QtError::InvalidEnvelope)
    }

    /// Resposta `Fichas` do relay: verifica a prova DLEQ com a chave do mes, tira a cegueira e
    /// guarda as fichas na base cifrada. Devolve quantas ficaram. Prova invalida: nenhuma.
    pub fn fichas_accept(&self, issued: FichasIssued) -> Result<u32, QtError> {
        let Some((req, _)) = crate::client::relock(&self.fichas).pending.take() else { return Err(bad("fichas")) };
        if issued.epoch != req.epoch() {
            // O relay esta noutro mes (relogio deste telemovel): o proximo pedido vai para esse, so
            // se for plausivel (o vizinho, a menos de 48 h da viragem). Outro qualquer: ignorado.
            crate::client::relock(&self.fichas).epoch_hint = Some(issued.epoch).filter(|e| epoch_plausible(*e));
            return Ok(0);
        }
        // Particao de chaves: o emissor so vale com a chave publicada no site para este mes.
        let published = {
            let c = self.lock()?;
            published_key(&c, issued.epoch)?
        };
        if issued.granted > 0 && published.is_none_or(|k| k[..] != issued.public_key[..]) {
            return Err(QtError::Crypto);
        }
        let fichas = req.finalize(&issued.public_key, &issued.evaluated, &issued.proof).map_err(|_| QtError::Crypto)?;
        if fichas.len() != issued.granted as usize {
            return Err(QtError::InvalidEnvelope);
        }
        let rows: Vec<(u32, [u8; 32], Vec<u8>)> = fichas.iter().map(|f| (f.epoch, f.nonce, f.to_bytes().to_vec())).collect();
        let c = self.lock()?;
        let tx = c.unchecked_transaction()?;
        st::fichas_put(&tx, &rows)?;
        let mut rem = issued.epoch.to_be_bytes().to_vec();
        rem.extend_from_slice(&issued.remaining.to_be_bytes());
        st::kv_set(&tx, KV_REMAINING, &rem)?;
        if issued.remaining == 0 || issued.granted == 0 {
            st::kv_set(&tx, KV_DONE, &issued.epoch.to_be_bytes())?;
        }
        tx.commit()?;
        Ok(fichas.len() as u32)
    }

    /// O relay nao emitiu (relay antigo, fichas desligadas, conta sem registo ou erro): so se
    /// volta a pedir daqui a umas horas.
    pub fn fichas_refused(&self) -> Result<(), QtError> {
        crate::client::relock(&self.fichas).pending = None;
        let c = self.lock()?;
        st::kv_set(&c, KV_RETRY_AT, &(unix_now() + RETRY_AFTER_SECS).to_be_bytes())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qt_core::fichas::Issuer;

    const SEED: [u8; 32] = [0x31; 32];

    fn client(name: &str) -> std::sync::Arc<QtClient> {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = format!("/tmp/qt_fichas_{name}_{}_{n}.sqlite", std::process::id());
        let _ = std::fs::remove_file(&path);
        let c = QtClient::open(path, vec![0x42; 32]).unwrap();
        c.create_identity().unwrap();
        assert_eq!(c.fichas_set_published_keys(site(&SEED, &[current_epoch(), 209901])).unwrap(), 2);
        c
    }

    /// O ficheiro do site (`estado.json`) com as chaves destes meses.
    fn site(seed: &[u8; 32], epochs: &[u32]) -> Vec<u8> {
        let mut m = serde_json::Map::new();
        for e in epochs {
            let k: String = Issuer::derive(seed, *e).unwrap().public_key().iter().map(|b| format!("{b:02x}")).collect();
            m.insert(format!("{e:06}"), serde_json::Value::String(k));
        }
        serde_json::to_vec(&serde_json::json!({"versao": 1, "fichas": m})).unwrap()
    }

    /// O relay: avalia o pedido com a chave do mes (ate `limit` fichas).
    fn relay_answer(req: &[u8], limit: usize, remaining_after: u32) -> FichasIssued {
        let v: serde_json::Value = serde_json::from_slice(req).unwrap();
        let epoch = v["IssueFichas"]["epoch"].as_u64().unwrap() as u32;
        let blinded = base64::engine::general_purpose::STANDARD.decode(v["IssueFichas"]["blinded"].as_str().unwrap()).unwrap();
        let iss = Issuer::derive(&SEED, epoch).unwrap();
        let k = (blinded.len() / 32).min(limit);
        let (evaluated, proof) = if k == 0 { (Vec::new(), Vec::new()) } else { let (e, p) = iss.issue(&blinded[..k * 32]).unwrap(); (e, p.to_vec()) };
        FichasIssued { epoch, public_key: iss.public_key().to_vec(), evaluated, proof, granted: k as u32, remaining: remaining_after, per_month: 300 }
    }

    #[test]
    fn pede_guarda_cifradas_e_nao_pede_outra_vez_no_mesmo_mes() {
        let c = client("pede");
        let s = c.fichas_status().unwrap();
        assert_eq!((s.epoch, s.left, s.due), (current_epoch(), 0, true));
        let req = c.relay_request_fichas().unwrap().expect("pedido");
        // Um pedido de cada vez.
        assert!(c.relay_request_fichas().unwrap().is_none());
        assert_eq!(c.fichas_accept(relay_answer(&req, 300, 0)).unwrap(), 300);
        let s = c.fichas_status().unwrap();
        assert_eq!((s.left, s.due), (300, false), "o relay disse que acabaram este mes");
        assert!(c.relay_request_fichas().unwrap().is_none());
        // Na base cifrada, validas para a chave do mes.
        let iss = Issuer::derive(&SEED, current_epoch()).unwrap();
        let rows = st::fichas_load(&c.lock().unwrap(), current_epoch()).unwrap();
        assert_eq!(rows.len(), 300);
        assert!(rows.iter().map(|b| Ficha::from_bytes(b).unwrap()).all(|f| iss.verify(f.epoch, &f.nonce, &f.output)));
    }

    #[test]
    fn quota_maior_pede_em_varios_lotes_ate_ao_teto() {
        let c = client("lotes");
        let req = c.relay_request_fichas().unwrap().unwrap();
        // Teto de 600: o relay da 300 e diz que faltam 300.
        assert_eq!(c.fichas_accept(relay_answer(&req, 300, 300)).unwrap(), 300);
        assert!(c.fichas_status().unwrap().due);
        let req = c.relay_request_fichas().unwrap().unwrap();
        assert_eq!(c.fichas_accept(relay_answer(&req, 300, 0)).unwrap(), 300);
        assert_eq!(c.fichas_status().unwrap().left, 600);
        assert!(!c.fichas_status().unwrap().due);
    }

    #[test]
    fn prova_invalida_nada_fica_e_relay_que_nao_emite_espera() {
        let c = client("prova");
        let req = c.relay_request_fichas().unwrap().unwrap();
        let mut a = relay_answer(&req, 5, 0);
        a.public_key = Issuer::derive(&[9u8; 32], a.epoch).unwrap().public_key().to_vec();
        assert!(c.fichas_accept(a).is_err(), "chave diferente da da prova");
        assert_eq!(c.fichas_status().unwrap().left, 0);
        // Resposta sem pedido: recusada.
        assert!(c.fichas_accept(relay_answer(&req, 5, 0)).is_err());
        // Relay antigo ou sem fichas: so volta a pedir daqui a umas horas.
        c.relay_request_fichas().unwrap().unwrap();
        c.fichas_refused().unwrap();
        assert!(!c.fichas_status().unwrap().due);
        assert!(c.relay_request_fichas().unwrap().is_none());
    }

    #[test]
    fn de_outro_mes_sao_apagadas_e_o_relay_corrige_o_mes() {
        let c = client("mes");
        {
            let db = c.lock().unwrap();
            st::fichas_put(&db, &[(190001, [1u8; 32], vec![0u8; qt_core::fichas::FICHA_LEN])]).unwrap();
        }
        assert_eq!(c.fichas_status().unwrap().left, 0);
        assert_eq!(st::fichas_count(&c.lock().unwrap(), 190001).unwrap(), 0, "o mes acabou: apagada");
        // O relay responde com um mes que nao e plausivel (longe deste): nada dado, e o proximo
        // pedido continua no mes deste telemovel (um relay que desse a cada conta um mes
        // diferente separava-as em grupos). O mes vizinho perto da viragem segue-se: ver
        // `qt_core::fichas::tests::mes_do_relay_so_o_proprio_ou_o_vizinho_na_viragem`.
        let req = c.relay_request_fichas().unwrap().unwrap();
        let mut a = relay_answer(&req, 0, 300);
        a.epoch = 209901;
        assert_eq!(c.fichas_accept(a).unwrap(), 0);
        let req = c.relay_request_fichas().unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&req).unwrap();
        assert_eq!(v["IssueFichas"]["epoch"].as_u64(), Some(current_epoch() as u64));
        assert_eq!(c.fichas_status().unwrap().epoch, current_epoch());
    }

    /// Fichas de um mes longe deste (o relay disse esse mes so a esta conta): nem as chaves desse
    /// mes nem as fichas vao para o motor, mesmo publicadas no site.
    #[cfg(feature = "calls")]
    #[test]
    fn so_meses_plausiveis_vao_para_o_motor() {
        let c = client("plausivel");
        {
            let iss = Issuer::derive(&SEED, 209901).unwrap();
            let (req, blinded) = IssueRequest::new(209901, 2).unwrap();
            let (ev, proof) = iss.issue(&blinded).unwrap();
            let f = req.finalize(&iss.public_key(), &ev, &proof).unwrap();
            let rows: Vec<_> = f.iter().map(|x| (x.epoch, x.nonce, x.to_bytes().to_vec())).collect();
            st::fichas_put(&c.lock().unwrap(), &rows).unwrap();
        }
        // O site publica 209901 (ver `client`), mas nao e o mes de agora.
        crate::client::relock(&c.fichas).epoch_hint = Some(209901);
        let (w, published) = wallet_for_call(&c).unwrap();
        assert_eq!(w.left(), 0);
        assert!(published.iter().all(|(e, _)| *e == current_epoch()), "{published:?}");
    }

    /// Particao de chaves na EMISSAO: o relay responde com uma chave (com prova valida para ela)
    /// que nao e a publicada no site. Recusa: nenhuma ficha fica.
    #[test]
    fn chave_diferente_da_publicada_na_emissao_e_recusada() {
        let c = client("particao");
        let req = c.relay_request_fichas().unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&req).unwrap();
        let blinded = base64::engine::general_purpose::STANDARD.decode(v["IssueFichas"]["blinded"].as_str().unwrap()).unwrap();
        let other = Issuer::derive(&[0x99; 32], current_epoch()).unwrap();
        let (evaluated, proof) = other.issue(&blinded[..5 * 32]).unwrap();
        let a = FichasIssued { epoch: current_epoch(), public_key: other.public_key().to_vec(), evaluated, proof: proof.to_vec(), granted: 5, remaining: 0, per_month: 300 };
        assert!(c.fichas_accept(a).is_err());
        assert_eq!(c.fichas_status().unwrap().left, 0);
    }

    /// Site indisponivel desde sempre (nenhuma chave publicada): nada se pede (falha fechada).
    #[test]
    fn sem_chave_do_site_nada_se_pede() {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = format!("/tmp/qt_fichas_semsite_{}_{}.sqlite", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
        let _ = std::fs::remove_file(&path);
        let c = QtClient::open(path, vec![0x43; 32]).unwrap();
        c.create_identity().unwrap();
        let s = c.fichas_status().unwrap();
        assert_eq!((s.published, s.due), (false, false));
        assert!(c.relay_request_fichas().unwrap().is_none());
        // Ficheiros invalidos nao contam (e nao apagam o que havia).
        for bad in [&b"nada"[..], br#"{"fichas":{}}"#, br#"{"fichas":{"202613":"00"}}"#, br#"{"fichas":{"202610":"zz"}}"#] {
            assert!(c.fichas_set_published_keys(bad.to_vec()).is_err());
        }
        assert!(!c.fichas_status().unwrap().published);
        // Chegou a chave do site: ja pode pedir.
        c.fichas_set_published_keys(site(&SEED, &[current_epoch()])).unwrap();
        assert!(c.fichas_status().unwrap().due);
        assert!(c.fichas_set_published_keys(b"lixo".to_vec()).is_err());
        assert!(c.fichas_status().unwrap().published, "um ficheiro estragado nao apaga as chaves boas");
    }

    #[cfg(feature = "calls")]
    #[test]
    fn carteira_da_chamada_e_as_usadas_saem_da_base() {
        let c = client("carteira");
        let req = c.relay_request_fichas().unwrap().unwrap();
        c.fichas_accept(relay_answer(&req, 3, 0)).unwrap();
        let (w, published) = wallet_for_call(&c).unwrap();
        assert_eq!(w.left(), 3);
        assert!(published.contains(&(current_epoch(), Issuer::derive(&SEED, current_epoch()).unwrap().public_key())));
        let iss = Issuer::derive(&SEED, current_epoch()).unwrap();
        let f = qt_call::fichas::Wallet::next(&*w, current_epoch(), &iss.public_key()).unwrap();
        qt_call::fichas::Wallet::used(&*w, &f.nonce);
        assert_eq!(c.fichas_status().unwrap().left, 2, "a usada saiu da base");
        // O site passa a publicar outra chave para este mes: as fichas guardadas nao vao para o motor.
        c.fichas_set_published_keys(site(&[0x55; 32], &[current_epoch()])).unwrap();
        assert_eq!(wallet_for_call(&c).unwrap().0.left(), 0);
    }
}
