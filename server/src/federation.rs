//! Federacao FECHADA de relays (fase 1: descoberta + registo unico).
//!
//! So relays da lista `/etc/qt/federacao.json` (escrita pelo operador) falam entre si.
//! O ID das contas nao muda (a chave Ed25519): cada relay pergunta aos outros onde esta
//! uma conta e guarda a resposta EM MEMORIA (nunca no Redis, que vai para as copias).
//! Uma conta so existe num relay: ao registar uma conta nova, o relay reserva-a nos outros.
//!
//! Ligacao: `wss://<par>/fed` (HAProxy, so dos IPs dos pares) -> `127.0.0.1:9446`. Noise_XX
//! com a chave FIXADA dos dois lados: quem liga confirma o pin do par; quem recebe so aceita
//! chaves da lista. Cada relay abre UMA ligacao a cada par para as SUAS perguntas (pedidos
//! numerados, varios ao mesmo tempo); as respostas vem pela mesma ligacao.
//!
//! Par que nao responde ao registar (decisao do Nuno, A1): regista na mesma e marca a conta
//! para verificar (`fed:verificar`). Quando o par volta: se ele ja a tinha, este relay larga-a;
//! se os dois a registaram sem verificar, fica no relay de nome menor.
//!
//! Registos: so o nome dos pares e contagens. Nunca IDs de contas.

use crate::protocol::{is_valid_user_id, UserId};
use crate::store::Store;
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Semaphore};

/// Versao do protocolo entre relays.
pub const VERSAO: u32 = 1;
/// Reserva de uma conta nova (para outro relay ou para um registo deste).
pub const RESERVA: Duration = Duration::from_secs(60);
/// Tempo maximo de uma pergunta a um par (ligacao ja aberta).
pub const TEMPO_PERGUNTA: Duration = Duration::from_secs(2);
/// "A conta esta no relay X": guardado 24 horas.
pub const CACHE_POSITIVA: Duration = Duration::from_secs(24 * 3600);
/// "Nenhum relay tem a conta" (ou apagada/banida): 60 s, contra IDs inventados.
pub const CACHE_NEGATIVA: Duration = Duration::from_secs(60);
/// Entradas na cache, no maximo.
pub const CACHE_MAX: usize = 200_000;
/// IDs por pedido `OndeEsta`.
pub const MAX_IDS: usize = 100;
/// Perguntas em curso por par (as outras esperam dentro do `TEMPO_PERGUNTA`).
pub const EM_CURSO: usize = 64;
/// Ligacoes ao `/fed` por autenticar ao mesmo tempo.
const MAX_POR_AUTENTICAR: usize = 16;
/// Ping a cada 30 s; sem nada do par em 90 s, a ligacao e dada como morta.
const PING: Duration = Duration::from_secs(30);
const MORTA: Duration = Duration::from_secs(90);
/// Verificacao das contas registadas com um par em baixo.
const VERIFICAR_CADA: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Configuracao
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct FedConfig {
    /// Nome deste relay (ex.: "estocolmo").
    pub nome: String,
    #[serde(default)]
    pub pares: Vec<ParConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ParConfig {
    pub nome: String,
    /// `wss://relay-xx.sealerapp.com/fed` (ws:// so para loopback em testes).
    pub url: String,
    /// Chave publica Noise do par (o pin das apps), 64 hex.
    pub chave: String,
    /// IP publico do par: so para o HAProxy (`setup-ec2.sh`), o relay nao o usa.
    #[serde(default)]
    pub ip: Option<String>,
}

fn nome_valido(n: &str) -> bool {
    !n.is_empty() && n.len() <= 32 && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

impl FedConfig {
    /// Le o ficheiro. `Ok(None)` se nao existir (relay sozinho, como antes da federacao).
    pub fn ler(path: &std::path::Path) -> Result<Option<Self>, String> {
        let texto = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("nao foi possivel ler o ficheiro da federacao".into()),
        };
        let cfg: FedConfig = serde_json::from_str(&texto).map_err(|e| format!("ficheiro da federacao invalido: {e}"))?;
        cfg.validar()?;
        Ok(Some(cfg))
    }

    pub fn validar(&self) -> Result<(), String> {
        if !nome_valido(&self.nome) {
            return Err("nome do relay invalido (a-z, 0-9 e -, ate 32)".into());
        }
        let mut nomes = std::collections::HashSet::new();
        let mut chaves = std::collections::HashSet::new();
        for p in &self.pares {
            if !nome_valido(&p.nome) || p.nome == self.nome || !nomes.insert(p.nome.clone()) {
                return Err(format!("par com nome invalido ou repetido: {}", p.nome));
            }
            let Some(k) = hex32(&p.chave.to_ascii_lowercase()) else {
                return Err(format!("chave do par {} invalida (64 hex)", p.nome));
            };
            if !chaves.insert(k) {
                return Err(format!("chave do par {} repetida", p.nome));
            }
            qt_transport::ws_transport::check_url_policy(&p.url).map_err(|_| format!("url do par {} invalido (wss://)", p.nome))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Protocolo entre relays (JSON dentro do Noise)
// ---------------------------------------------------------------------------

/// O que um relay sabe de uma conta (so do seu proprio Redis).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Estado {
    Tenho,
    /// Registada aqui com um par em baixo; ainda por verificar.
    TenhoPorVerificar,
    Apagada,
    Banida,
    NaoTenho,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Pedido {
    /// Primeira mensagem de cada ligacao.
    Ola { nome: String, versao: u32 },
    /// Ate `MAX_IDS` contas. Resposta: `Aqui`, pela mesma ordem.
    OndeEsta { ids: Vec<UserId> },
    /// Conta nova a registar no relay que pergunta. Resposta: `Reserva`.
    Reservar { id: UserId },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Resposta {
    Ola { nome: String, versao: u32 },
    Aqui { estados: Vec<Estado> },
    /// `ok = false`: este relay tem a conta, a tem banida (`banida`) ou esta a registá-la.
    Reserva {
        ok: bool,
        #[serde(default)]
        banida: bool,
    },
    Erro { motivo: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct PedidoN {
    n: u64,
    p: Pedido,
}

#[derive(Debug, Serialize, Deserialize)]
struct RespostaN {
    n: u64,
    r: Resposta,
}

// ---------------------------------------------------------------------------
// Resultados para o resto do relay
// ---------------------------------------------------------------------------

/// Resultado de reservar uma conta nova nos pares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reserva {
    /// Pode registar. `por_verificar`: algum par nao respondeu (opcao A1).
    Livre { por_verificar: bool },
    /// Outro relay ja tem a conta (ou esta a registá-la agora).
    NoutroRelay,
    /// Banida noutro relay.
    Banida,
}

/// Onde esta uma conta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Onde {
    /// Neste relay.
    Aqui,
    /// No relay com este nome.
    Par(String),
    Banida,
    Apagada,
    /// Nenhum relay a tem.
    Nenhum,
    /// Nenhum dos que responderam a tem, mas algum par nao respondeu (nao vai para a cache).
    Incerto,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Origem {
    Local,
    Par(String),
}

struct Envio {
    p: Pedido,
    resp: oneshot::Sender<Resposta>,
}

pub struct Par {
    pub nome: String,
    url: String,
    chave: [u8; 32],
    tx: mpsc::Sender<Envio>,
    ligado: AtomicBool,
    vagas: Semaphore,
}

impl Par {
    pub fn ligado(&self) -> bool {
        self.ligado.load(Ordering::Relaxed)
    }
}

pub struct Federacao {
    pub nome: String,
    pares: Vec<Arc<Par>>,
    store: Arc<Store>,
    reservas: Mutex<HashMap<UserId, (Origem, Instant)>>,
    cache: Mutex<HashMap<UserId, (Onde, Instant)>>,
}

impl std::fmt::Debug for Federacao {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pares: Vec<&str> = self.pares.iter().map(|p| p.nome.as_str()).collect();
        f.debug_struct("Federacao").field("nome", &self.nome).field("pares", &pares).finish()
    }
}

impl Federacao {
    /// Arranca a federacao: ouve os pares em `ouvinte` (o `/fed` do HAProxy), liga-se a cada
    /// par e verifica de minuto a minuto as contas registadas com um par em baixo.
    /// `chave_privada`: a chave Noise do relay (a mesma das apps).
    pub fn arrancar(cfg: FedConfig, store: Arc<Store>, chave_privada: Vec<u8>, ouvinte: TcpListener) -> Arc<Self> {
        let mut filas = Vec::new();
        let pares = cfg
            .pares
            .iter()
            .map(|p| {
                let (tx, rx) = mpsc::channel(1024);
                filas.push(rx);
                Arc::new(Par {
                    nome: p.nome.clone(),
                    url: p.url.clone(),
                    chave: hex32(&p.chave.to_ascii_lowercase()).expect("validado"),
                    tx,
                    ligado: AtomicBool::new(false),
                    vagas: Semaphore::new(EM_CURSO),
                })
            })
            .collect::<Vec<_>>();
        let fed = Arc::new(Federacao {
            nome: cfg.nome,
            pares,
            store,
            reservas: Mutex::new(HashMap::new()),
            cache: Mutex::new(HashMap::new()),
        });
        for (par, rx) in fed.pares.iter().cloned().zip(filas) {
            tokio::spawn(manter_ligacao(fed.clone(), par, rx, chave_privada.clone()));
        }
        tokio::spawn(ouvir(fed.clone(), ouvinte, chave_privada));
        let f = fed.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(VERIFICAR_CADA);
            tick.tick().await;
            loop {
                tick.tick().await;
                f.verificar_pendentes().await;
            }
        });
        fed
    }

    pub fn pares(&self) -> &[Arc<Par>] {
        &self.pares
    }

    /// Pergunta a um par. `None`: desligado, ocupado, sem resposta a tempo ou erro.
    async fn perguntar(&self, par: &Par, p: Pedido) -> Option<Resposta> {
        if !par.ligado() {
            return None;
        }
        let pedido = async {
            let _vaga = par.vagas.acquire().await.ok()?;
            let (tx, rx) = oneshot::channel();
            par.tx.try_send(Envio { p, resp: tx }).ok()?;
            rx.await.ok()
        };
        match tokio::time::timeout(TEMPO_PERGUNTA, pedido).await {
            Ok(Some(Resposta::Erro { .. })) | Ok(None) | Err(_) => None,
            Ok(Some(r)) => Some(r),
        }
    }

    /// Estados destas contas em cada par (`None` = esse par nao respondeu).
    async fn estados_nos_pares(&self, ids: &[UserId]) -> Vec<(String, Option<Vec<Estado>>)> {
        let perguntas = self.pares.iter().map(|par| async move {
            let mut todos = Vec::with_capacity(ids.len());
            for lote in ids.chunks(MAX_IDS) {
                match self.perguntar(par, Pedido::OndeEsta { ids: lote.to_vec() }).await {
                    Some(Resposta::Aqui { estados }) if estados.len() == lote.len() => todos.extend(estados),
                    _ => return (par.nome.clone(), None),
                }
            }
            (par.nome.clone(), Some(todos))
        });
        join_all(perguntas).await
    }

    // ----- Registo unico -----

    /// Antes de registar uma conta NOVA (sem bundle neste relay): reserva-a em todos os pares.
    /// Chamar `libertar` depois do registo (ou da recusa).
    pub async fn reservar(&self, id: &str) -> Reserva {
        {
            let mut r = self.reservas.lock().unwrap();
            let agora = Instant::now();
            r.retain(|_, (_, t)| agora.duration_since(*t) < RESERVA);
            match r.get(id) {
                Some((Origem::Par(_), _)) => return Reserva::NoutroRelay,
                _ => {
                    r.insert(id.to_string(), (Origem::Local, agora));
                }
            }
        }
        let respostas = join_all(self.pares.iter().map(|p| self.perguntar(p, Pedido::Reservar { id: id.to_string() }))).await;
        let mut resultado = Reserva::Livre { por_verificar: false };
        for r in respostas {
            match r {
                Some(Resposta::Reserva { banida: true, .. }) => return self.recusar(id, Reserva::Banida),
                Some(Resposta::Reserva { ok: false, .. }) => resultado = Reserva::NoutroRelay,
                Some(Resposta::Reserva { ok: true, .. }) => {}
                _ => {
                    if resultado == (Reserva::Livre { por_verificar: false }) {
                        resultado = Reserva::Livre { por_verificar: true };
                    }
                }
            }
        }
        if resultado == Reserva::NoutroRelay {
            return self.recusar(id, resultado);
        }
        resultado
    }

    fn recusar(&self, id: &str, r: Reserva) -> Reserva {
        self.libertar(id);
        r
    }

    /// Acaba a reserva local (a conta ja tem bundle aqui, ou o registo foi recusado).
    pub fn libertar(&self, id: &str) {
        let mut r = self.reservas.lock().unwrap();
        if matches!(r.get(id), Some((Origem::Local, _))) {
            r.remove(id);
        }
    }

    /// Outro relay quer registar `id`.
    async fn reserva_pedida(&self, par: &str, id: &str) -> Resposta {
        if !is_valid_user_id(id) {
            return Resposta::Erro { motivo: "id invalido".into() };
        }
        match self.store.fed_estado(id).await {
            Ok(Estado::Tenho | Estado::TenhoPorVerificar) => return Resposta::Reserva { ok: false, banida: false },
            Ok(Estado::Banida) => return Resposta::Reserva { ok: false, banida: true },
            Ok(_) => {}
            Err(_) => return Resposta::Erro { motivo: "erro interno".into() },
        }
        let mut r = self.reservas.lock().unwrap();
        let agora = Instant::now();
        let origem = Origem::Par(par.to_string());
        match r.get(id) {
            Some((o, t)) if agora.duration_since(*t) < RESERVA && *o != origem => Resposta::Reserva { ok: false, banida: false },
            _ => {
                r.insert(id.to_string(), (origem, agora));
                Resposta::Reserva { ok: true, banida: false }
            }
        }
    }

    // ----- Descoberta -----

    /// Onde esta esta conta: primeiro o Redis local, depois a cache, depois os pares.
    pub async fn onde_esta(&self, id: &str) -> Onde {
        self.onde_estao(&[id.to_string()]).await.pop().unwrap_or(Onde::Incerto)
    }

    /// Como `onde_esta`, para varias contas (um pedido por par por cada `MAX_IDS`).
    pub async fn onde_estao(&self, ids: &[UserId]) -> Vec<Onde> {
        let mut out: Vec<Option<Onde>> = vec![None; ids.len()];
        let mut perguntar = Vec::new();
        let mut locais = Vec::with_capacity(ids.len());
        for (i, id) in ids.iter().enumerate() {
            let local = if is_valid_user_id(id) { self.store.fed_estado(id).await.ok() } else { Some(Estado::NaoTenho) };
            if matches!(local, Some(Estado::Tenho | Estado::TenhoPorVerificar)) {
                out[i] = Some(Onde::Aqui);
            } else if let Some(c) = self.da_cache(id) {
                out[i] = Some(c);
            } else if is_valid_user_id(id) {
                perguntar.push(i);
            } else {
                out[i] = Some(Onde::Nenhum);
            }
            locais.push(local);
        }
        if !perguntar.is_empty() {
            let lista: Vec<UserId> = perguntar.iter().map(|&i| ids[i].clone()).collect();
            let respostas = self.estados_nos_pares(&lista).await;
            for (k, &i) in perguntar.iter().enumerate() {
                let mut sem_resposta = locais[i].is_none();
                let mut onde = None;
                let (mut banida, mut apagada) = (locais[i] == Some(Estado::Banida), locais[i] == Some(Estado::Apagada));
                for (nome, estados) in &respostas {
                    match estados.as_ref().map(|e| e[k]) {
                        Some(Estado::Tenho | Estado::TenhoPorVerificar) => onde = Some(Onde::Par(nome.clone())),
                        Some(Estado::Banida) => banida = true,
                        Some(Estado::Apagada) => apagada = true,
                        Some(Estado::NaoTenho) => {}
                        None => sem_resposta = true,
                    }
                }
                let resultado = match onde {
                    Some(o) => o,
                    None if banida => Onde::Banida,
                    None if sem_resposta => Onde::Incerto,
                    None if apagada => Onde::Apagada,
                    None => Onde::Nenhum,
                };
                if resultado != Onde::Incerto {
                    self.para_cache(&ids[i], resultado.clone());
                }
                out[i] = Some(resultado);
            }
        }
        out.into_iter().map(|o| o.unwrap_or(Onde::Incerto)).collect()
    }

    /// Esquece o que sabia desta conta (ex.: o relay de casa disse que ja nao a tem).
    pub fn esquecer(&self, id: &str) {
        self.cache.lock().unwrap().remove(id);
    }

    fn da_cache(&self, id: &str) -> Option<Onde> {
        let mut c = self.cache.lock().unwrap();
        let (onde, ate) = c.get(id)?.clone();
        if Instant::now() < ate {
            Some(onde)
        } else {
            c.remove(id);
            None
        }
    }

    fn para_cache(&self, id: &str, onde: Onde) {
        let dura = if matches!(onde, Onde::Par(_)) { CACHE_POSITIVA } else { CACHE_NEGATIVA };
        let mut c = self.cache.lock().unwrap();
        if c.len() >= CACHE_MAX {
            let agora = Instant::now();
            c.retain(|_, (_, ate)| *ate > agora);
            if c.len() >= CACHE_MAX {
                c.clear();
            }
        }
        c.insert(id.to_string(), (onde, Instant::now() + dura));
    }

    // ----- Verificacao das contas registadas com um par em baixo (A1) -----

    /// Uma volta: ate 500 contas por verificar. Devolve (verificadas, largadas).
    pub async fn verificar_pendentes(&self) -> (usize, usize) {
        let Ok(ids) = self.store.fed_por_verificar(500).await else { return (0, 0) };
        if ids.is_empty() {
            return (0, 0);
        }
        let respostas = self.estados_nos_pares(&ids).await;
        let (mut verificadas, mut largadas) = (0, 0);
        for (k, id) in ids.iter().enumerate() {
            match self.store.fed_estado(id).await {
                Ok(Estado::TenhoPorVerificar) => {}
                Ok(_) => {
                    // Ja nao tem a conta (apagada, expirou, banida): so limpa a marca.
                    let _ = self.store.fed_verificado(id).await;
                    continue;
                }
                Err(_) => continue,
            }
            let mut decisao = Some(false); // Some(largar?) ; None = esperar
            for (nome, estados) in &respostas {
                match estados.as_ref().map(|e| e[k]) {
                    None => {
                        if decisao == Some(false) {
                            decisao = None;
                        }
                    }
                    Some(Estado::Tenho) => decisao = Some(true),
                    // Os dois registaram sem verificar: fica no relay de nome menor.
                    Some(Estado::TenhoPorVerificar) if nome.as_str() < self.nome.as_str() => decisao = Some(true),
                    Some(_) => {}
                }
                if decisao == Some(true) {
                    break;
                }
            }
            match decisao {
                Some(true) if self.store.fed_largar(id).await.is_ok() => largadas += 1,
                Some(false) if self.store.fed_verificado(id).await.is_ok() => verificadas += 1,
                _ => {}
            }
        }
        if verificadas + largadas > 0 {
            println!("federacao: {verificadas} contas verificadas, {largadas} largadas (estavam noutro relay)");
        }
        (verificadas, largadas)
    }

    /// Responde a um pedido de um par (ja autenticado como `par`).
    async fn responder(&self, par: &str, p: Pedido) -> Resposta {
        match p {
            Pedido::Ola { .. } => Resposta::Erro { motivo: "ola repetido".into() },
            Pedido::Reservar { id } => self.reserva_pedida(par, &id).await,
            Pedido::OndeEsta { ids } => {
                if ids.len() > MAX_IDS {
                    return Resposta::Erro { motivo: "demasiados ids".into() };
                }
                let mut estados = Vec::with_capacity(ids.len());
                for id in &ids {
                    if !is_valid_user_id(id) {
                        estados.push(Estado::NaoTenho);
                        continue;
                    }
                    match self.store.fed_estado(id).await {
                        Ok(e) => estados.push(e),
                        Err(_) => return Resposta::Erro { motivo: "erro interno".into() },
                    }
                }
                Resposta::Aqui { estados }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Ligacoes
// ---------------------------------------------------------------------------

fn enc<T: Serialize>(v: &T) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_default()
}

/// Liga-se a um par e mantem a ligacao (volta a ligar com espera de 1 s a 60 s).
async fn manter_ligacao(fed: Arc<Federacao>, par: Arc<Par>, mut rx: mpsc::Receiver<Envio>, chave: Vec<u8>) {
    use qt_transport::ws_transport::{self as ws, ClientOptions, Incoming};
    let mut espera = Duration::from_secs(1);
    loop {
        let opts = ClientOptions { connect_timeout: Duration::from_secs(10), expected_server_static: Some(par.chave.to_vec()) };
        let ligacao = async {
            let (mut s, mut noise) = ws::client_connect_with(&par.url, &chave, &opts).await.ok()?;
            let ola = PedidoN { n: 0, p: Pedido::Ola { nome: fed.nome.clone(), versao: VERSAO } };
            ws::send_encrypted(&mut s, &mut noise, &enc(&ola)).await.ok()?;
            let r = ws::receive_encrypted_timeout(&mut s, &mut noise, Duration::from_secs(5)).await.ok()?;
            match serde_json::from_slice::<RespostaN>(&r).ok()? {
                RespostaN { n: 0, r: Resposta::Ola { nome, versao } } if nome == par.nome && versao >= 1 => Some((s, noise)),
                _ => {
                    eprintln!("federacao: o par {} nao se identificou como esperado", par.nome);
                    None
                }
            }
        };
        let Some((mut s, mut noise)) = ligacao.await else {
            // Sem ligacao: as perguntas que cheguem entretanto falham logo.
            let ate = tokio::time::sleep(espera);
            tokio::pin!(ate);
            loop {
                tokio::select! {
                    _ = &mut ate => break,
                    e = rx.recv() => if e.is_none() { return },
                }
            }
            espera = (espera * 2).min(Duration::from_secs(60));
            continue;
        };
        espera = Duration::from_secs(1);
        par.ligado.store(true, Ordering::Relaxed);
        println!("federacao: ligado a {}", par.nome);
        let mut pendentes: HashMap<u64, oneshot::Sender<Resposta>> = HashMap::new();
        let mut n: u64 = 0;
        let mut visto = Instant::now();
        let mut ping = tokio::time::interval(PING);
        ping.tick().await;
        loop {
            tokio::select! {
                e = rx.recv() => {
                    let Some(Envio { p, resp }) = e else { return };
                    n += 1;
                    if ws::send_encrypted(&mut s, &mut noise, &enc(&PedidoN { n, p })).await.is_err() {
                        break;
                    }
                    pendentes.retain(|_, tx| !tx.is_closed());
                    pendentes.insert(n, resp);
                }
                i = ws::next_incoming(&mut s) => match i {
                    Incoming::Data(f) => {
                        visto = Instant::now();
                        let Ok(texto) = noise.decrypt(&f) else { break };
                        if let Ok(RespostaN { n, r }) = serde_json::from_slice::<RespostaN>(&texto) {
                            if let Some(tx) = pendentes.remove(&n) {
                                let _ = tx.send(r);
                            }
                        }
                    }
                    Incoming::Pong(_) | Incoming::Control => visto = Instant::now(),
                    Incoming::Closed => break,
                },
                _ = ping.tick() => {
                    if visto.elapsed() > MORTA || ws::send_ping(&mut s, b"qt-fed".to_vec()).await.is_err() {
                        break;
                    }
                }
            }
        }
        par.ligado.store(false, Ordering::Relaxed);
        drop(pendentes);
        ws::close_gracefully(&mut s).await;
        println!("federacao: ligacao a {} caiu", par.nome);
    }
}

/// Aceita ligacoes dos pares (`/fed`, ja sem TLS, vindas do HAProxy).
async fn ouvir(fed: Arc<Federacao>, ouvinte: TcpListener, chave: Vec<u8>) {
    let vagas = Arc::new(Semaphore::new(MAX_POR_AUTENTICAR));
    loop {
        let Ok((tcp, _)) = ouvinte.accept().await else {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        let Ok(vaga) = vagas.clone().try_acquire_owned() else { continue };
        let (fed, chave) = (fed.clone(), chave.clone());
        tokio::spawn(async move {
            let _ = tcp.set_nodelay(true);
            if let Some((s, noise, nome)) = autenticar(&fed, tcp, &chave).await {
                drop(vaga);
                atender(fed, s, noise, nome).await;
            }
        });
    }
}

type Servidor = qt_transport::ws_transport::ServerWsStream;

/// Handshake Noise + chave na lista + `Ola` com o nome certo.
async fn autenticar(
    fed: &Federacao,
    tcp: tokio::net::TcpStream,
    chave: &[u8],
) -> Option<(Servidor, qt_transport::NoiseTransport, String)> {
    use qt_transport::ws_transport as ws;
    let (mut s, mut noise) = ws::server_accept_with(tcp, chave, Duration::from_secs(5)).await.ok()?;
    let remota = noise.remote_static_public_key().map(|k| k.to_vec());
    let Some(par) = fed.pares.iter().find(|p| remota.as_deref() == Some(p.chave.as_slice())) else {
        ws::close_gracefully(&mut s).await;
        return None;
    };
    let primeiro = ws::receive_encrypted_timeout(&mut s, &mut noise, Duration::from_secs(5)).await.ok()?;
    match serde_json::from_slice::<PedidoN>(&primeiro).ok()? {
        PedidoN { n: 0, p: Pedido::Ola { nome, versao } } if nome == par.nome && versao >= 1 => {}
        _ => {
            ws::close_gracefully(&mut s).await;
            return None;
        }
    }
    let ola = RespostaN { n: 0, r: Resposta::Ola { nome: fed.nome.clone(), versao: VERSAO } };
    ws::send_encrypted(&mut s, &mut noise, &enc(&ola)).await.ok()?;
    Some((s, noise, par.nome.clone()))
}

/// Responde aos pedidos de um par. Cada pedido corre a parte (ate `EM_CURSO` ao mesmo tempo);
/// as respostas saem pela ordem em que ficam prontas, com o numero do pedido.
async fn atender(fed: Arc<Federacao>, mut s: Servidor, mut noise: qt_transport::NoiseTransport, par: String) {
    use qt_transport::ws_transport::{self as ws, Incoming};
    let (tx, mut rx) = mpsc::channel::<RespostaN>(EM_CURSO * 2);
    let vagas = Arc::new(Semaphore::new(EM_CURSO));
    let par = Arc::new(par);
    loop {
        tokio::select! {
            i = tokio::time::timeout(MORTA, ws::next_incoming(&mut s)) => match i {
                Ok(Incoming::Data(f)) => {
                    let Ok(texto) = noise.decrypt(&f) else { break };
                    let Ok(PedidoN { n, p }) = serde_json::from_slice::<PedidoN>(&texto) else {
                        continue;
                    };
                    // Mais de `EM_CURSO` ao mesmo tempo (quem pergunta nunca passa disso): recusa
                    // logo, sem parar de ler a ligacao.
                    let Ok(vaga) = vagas.clone().try_acquire_owned() else {
                        let r = RespostaN { n, r: Resposta::Erro { motivo: "ocupado".into() } };
                        if ws::send_encrypted(&mut s, &mut noise, &enc(&r)).await.is_err() {
                            break;
                        }
                        continue;
                    };
                    let (fed, tx, par) = (fed.clone(), tx.clone(), par.clone());
                    tokio::spawn(async move {
                        let r = fed.responder(&par, p).await;
                        let _ = tx.send(RespostaN { n, r }).await;
                        drop(vaga);
                    });
                }
                Ok(Incoming::Control) | Ok(Incoming::Pong(_)) => {}
                Ok(Incoming::Closed) | Err(_) => break,
            },
            Some(r) = rx.recv() => {
                if ws::send_encrypted(&mut s, &mut noise, &enc(&r)).await.is_err() {
                    break;
                }
            }
        }
    }
    ws::close_gracefully(&mut s).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn par(nome: &str, chave: &str) -> ParConfig {
        ParConfig { nome: nome.into(), url: "wss://relay-es.sealerapp.com/fed".into(), chave: chave.into(), ip: None }
    }

    #[test]
    fn configuracao_validada() {
        let k = "ab".repeat(32);
        let ok = FedConfig { nome: "estocolmo".into(), pares: vec![par("espanha", &k)] };
        assert!(ok.validar().is_ok());
        assert!(FedConfig { nome: "Estocolmo".into(), pares: vec![] }.validar().is_err(), "maiusculas");
        assert!(FedConfig { nome: "estocolmo".into(), pares: vec![par("estocolmo", &k)] }.validar().is_err(), "o proprio");
        assert!(FedConfig { nome: "a".into(), pares: vec![par("b", &k), par("c", &k)] }.validar().is_err(), "chave repetida");
        assert!(FedConfig { nome: "a".into(), pares: vec![par("b", "12")] }.validar().is_err(), "chave curta");
        let mut http = par("b", &k);
        http.url = "ws://relay-es.sealerapp.com/fed".into();
        assert!(FedConfig { nome: "a".into(), pares: vec![http] }.validar().is_err(), "sem TLS");
    }

    #[test]
    fn formato_dos_pedidos() {
        let p = enc(&PedidoN { n: 3, p: Pedido::Reservar { id: "ab".into() } });
        assert_eq!(String::from_utf8(p).unwrap(), r#"{"n":3,"p":{"Reservar":{"id":"ab"}}}"#);
        let r: RespostaN = serde_json::from_str(r#"{"n":3,"r":{"Reserva":{"ok":true}}}"#).unwrap();
        assert!(matches!(r.r, Resposta::Reserva { ok: true, banida: false }));
    }
}
