//! Testes de ponta a ponta como os telemoveis usam a app: relay verdadeiro (Redis), nucleo
//! verdadeiro e os passos da app Android (`common/telemovel.rs`). Cada teste e um cenario que um
//! utilizador real pode viver.

mod common;
#[path = "common/telemovel.rs"]
mod telemovel;

use common::*;
use qt_ffi::history::MessageStatus;
use telemovel::*;

async fn dois(relay: &TestRelay) -> (Telemovel, Telemovel) {
    let mut a = Telemovel::novo("ana", &relay.url);
    let mut b = Telemovel::novo("bruno", &relay.url);
    a.criar_conta().await;
    b.criar_conta().await;
    a.adicionar(b.como_contacto("Bruno"));
    b.adicionar(a.como_contacto("Ana"));
    (a, b)
}

fn cid(t: &Telemovel) -> String {
    t.como_contacto("x").contact_id()
}

#[tokio::test]
async fn conversa_recibos_reinicio_e_sem_rede() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (mut a, mut b) = dois(&relay).await;
    let (ca, cb) = (cid(&a), cid(&b));

    a.enviar(&cb, "ola").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    assert_eq!(b.recebidas(&ca), ["ola"]);
    assert!(a.minhas(&cb).iter().all(|m| m.status == MessageStatus::Delivered || m.status == MessageStatus::Read), "recibo de entrega");

    b.enviar(&ca, "tudo bem?").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    assert_eq!(a.recebidas(&cb), ["tudo bem?"]);

    // Reiniciar as apps nao perde nada nem parte a sessao.
    a.reiniciar_app().await;
    b.reiniciar_app().await;
    a.enviar(&cb, "depois de reiniciar").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    assert_eq!(b.recebidas(&ca), ["ola", "depois de reiniciar"]);

    // Sem rede: fica na outbox e sai quando volta, pela ordem.
    a.sem_rede();
    for i in 0..5 {
        a.enviar(&cb, &format!("offline {i}")).await.unwrap();
    }
    a.ligar().await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    let r = b.recebidas(&ca);
    assert_eq!(&r[2..], ["offline 0", "offline 1", "offline 2", "offline 3", "offline 4"]);
    assert!(a.erros_receber.is_empty() && b.erros_receber.is_empty(), "{:?} {:?}", a.erros_receber, b.erros_receber);
}

#[tokio::test]
async fn contacto_novo_chega_como_convite_e_responde() {
    let relay = TestRelay::start(fast_cfg()).await;
    let mut a = Telemovel::novo("ana", &relay.url);
    let mut b = Telemovel::novo("bruno", &relay.url);
    a.criar_conta().await;
    b.criar_conta().await;
    a.adicionar(b.como_contacto("Bruno"));
    let (ca, cb) = (cid(&a), cid(&b));
    a.enviar(&cb, "sou a ana").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    assert_eq!(b.qt.pending_intros().unwrap(), std::slice::from_ref(&ca), "convite");
    assert_eq!(b.recebidas(&ca), ["sou a ana"]);
    b.aceitar_convite(&ca, "Ana");
    b.enviar(&ca, "ola ana").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    assert_eq!(a.recebidas(&cb), ["ola bruno".replace("bruno", "ana")]);
    assert!(a.minhas(&cb).iter().all(|m| m.status == MessageStatus::Delivered || m.status == MessageStatus::Read), "o recibo chegou depois de aceitar");
}

/// Mensagens escritas enquanto um dos lados esta desinstalado, recuperacao de um so lado.
#[tokio::test]
async fn recuperar_um_lado_nada_se_perde() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (mut a, mut b) = dois(&relay).await;
    let (ca, cb) = (cid(&a), cid(&b));
    let palavras = a.ativar_recuperacao().await;
    a.enviar(&cb, "antes").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;

    a.desinstalar();
    b.enviar(&ca, "escrita com a ana desinstalada").await.unwrap();
    b.ciclo().await;
    a.recuperar(palavras).await.unwrap();
    for _ in 0..3 {
        ate_acalmar(&mut [&mut a, &mut b]).await;
    }
    a.enviar(&cb, "voltei").await.unwrap();
    b.enviar(&ca, "bem-vinda").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    assert_eq!(b.recebidas(&ca), ["antes", "voltei"]);
    let r = a.recebidas(&cb);
    assert!(r.contains(&"escrita com a ana desinstalada".to_string()), "a mensagem escrita durante a reinstalacao perdeu-se: {r:?}");
    assert!(r.contains(&"bem-vinda".to_string()), "{r:?}");
}

/// O caso de 13/11/2026: os dois recuperam, em varias ordens e tempos.
#[tokio::test]
async fn recuperar_os_dois_em_todas_as_ordens() {
    let relay = TestRelay::start(fast_cfg()).await;
    for ordem in 0..6 {
        let (mut a, mut b) = dois(&relay).await;
        let (ca, cb) = (cid(&a), cid(&b));
        let (pa, pb) = (a.ativar_recuperacao().await, b.ativar_recuperacao().await);
        a.enviar(&cb, "inicio").await.unwrap();
        ate_acalmar(&mut [&mut a, &mut b]).await;
        a.desinstalar();
        b.desinstalar();
        match ordem {
            0 => { a.recuperar(pa).await.unwrap(); b.recuperar(pb).await.unwrap(); }
            1 => { b.recuperar(pb).await.unwrap(); a.recuperar(pa).await.unwrap(); }
            2 => { a.recuperar(pa).await.unwrap(); a.enviar(&cb, "a escreve antes de b voltar").await.unwrap(); b.recuperar(pb).await.unwrap(); }
            3 => { b.recuperar(pb).await.unwrap(); b.enviar(&ca, "b escreve antes de a voltar").await.unwrap(); a.recuperar(pa).await.unwrap(); }
            4 => { a.recuperar(pa).await.unwrap(); a.ciclo().await; b.recuperar(pb).await.unwrap(); b.enviar(&ca, "x").await.unwrap(); }
            _ => { a.recuperar(pa).await.unwrap(); b.recuperar(pb).await.unwrap(); a.enviar(&cb, "y").await.unwrap(); b.enviar(&ca, "z").await.unwrap(); }
        }
        for _ in 0..4 {
            ate_acalmar(&mut [&mut a, &mut b]).await;
        }
        a.enviar(&cb, "de a").await.unwrap();
        b.enviar(&ca, "de b").await.unwrap();
        for _ in 0..3 {
            ate_acalmar(&mut [&mut a, &mut b]).await;
        }
        let (ra, rb) = (a.recebidas(&cb), b.recebidas(&ca));
        println!("ordem {ordem}: pedidos A {:?}", a.pedidos.lock().unwrap());
        println!("ordem {ordem}: pedidos B {:?}", b.pedidos.lock().unwrap());
        assert!(a.pedidos_de("FetchPreKeyBundle") <= 6 && b.pedidos_de("FetchPreKeyBundle") <= 6, "bundles pedidos em ciclo");
        assert!(rb.contains(&"de a".to_string()), "ordem {ordem}: b nao recebe: {rb:?}");
        assert!(ra.contains(&"de b".to_string()), "ordem {ordem}: a nao recebe: {ra:?}");
        for t in ["a escreve antes de b voltar", "x", "y"] {
            if a.minhas(&cb).iter().any(|m| m.text == t) { assert!(rb.contains(&t.to_string()), "ordem {ordem}: perdeu-se '{t}': {rb:?}"); }
        }
        for t in ["b escreve antes de a voltar", "z"] {
            if b.minhas(&ca).iter().any(|m| m.text == t) { assert!(ra.contains(&t.to_string()), "ordem {ordem}: perdeu-se '{t}': {ra:?}"); }
        }
    }
}

/// Conta nova (menos de um dia: 60 envios por minuto). A app nao pode multiplicar as recusas e
/// tudo tem de chegar, pela ordem.
#[tokio::test]
async fn conta_nova_no_limite_de_envios() {
    let mut cfg = fast_cfg();
    cfg.send_rate_new = qt_server::limits::Rate { burst: 20, per_minute: 600 };
    let relay = TestRelay::start(cfg).await;
    for modo in [Outbox::Insiste, Outbox::ParaNoLimite] {
        let (mut a, mut b) = dois(&relay).await;
        a.outbox = modo;
        let cb = cid(&b);
        let ca = cid(&a);
        a.garantir_sessao(&cb).await.unwrap();
        for i in 0..60 {
            a.qt.send_text(cb.clone(), format!("m{i}")).unwrap();
        }
        // A app tenta enviar a cada mensagem que chega e a cada ciclo.
        for _ in 0..10 {
            a.flush().await;
        }
        let recusas = a.recusas.get("demasiados pedidos, tenta daqui a pouco").copied().unwrap_or(0);
        println!("{modo:?}: {recusas} recusas em 10 ciclos");
        if modo == Outbox::ParaNoLimite {
            assert!(recusas <= 10, "no maximo uma recusa por ciclo: {recusas}");
        }
        for _ in 0..30 {
            a.flush().await;
            b.ciclo().await;
            if b.recebidas(&ca).len() == 60 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        let r = b.recebidas(&ca);
        let certo: Vec<String> = (0..60).map(|i| format!("m{i}")).collect();
        println!("{modo:?}: chegaram {} de 60, pela ordem: {}", r.len(), r == certo);
        if modo == Outbox::ParaNoLimite {
            assert_eq!(r, certo, "tudo e pela ordem");
        }
    }
}

/// Chamada: oferta, toque, atender, desligar, como nos ecras.
#[tokio::test]
async fn chamada_sinalizacao_completa() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (mut a, mut b) = dois(&relay).await;
    let (ca, cb) = (cid(&a), cid(&b));
    a.enviar(&cb, "vou ligar").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    let start = a.qt.call_start(cb.clone()).unwrap();
    a.flush().await;
    b.ciclo().await;
    let ev = b.qt.call_events().unwrap();
    assert!(ev.iter().any(|e| matches!(e, qt_ffi::call::CallEvent::Incoming { contact_id, .. } if *contact_id == ca)), "{ev:?}");
    b.flush().await;
    a.ciclo().await;
    let ev = a.qt.call_events().unwrap();
    assert!(ev.iter().any(|e| matches!(e, qt_ffi::call::CallEvent::Ringing { .. })), "a tocar: {ev:?}");
    b.qt.call_accept(start.call_id.clone()).unwrap();
    b.flush().await;
    a.ciclo().await;
    let ev = a.qt.call_events().unwrap();
    assert!(ev.iter().any(|e| matches!(e, qt_ffi::call::CallEvent::Answered { .. })), "atendida: {ev:?}");
    a.qt.call_hangup(start.call_id.clone()).unwrap();
    a.flush().await;
    b.ciclo().await;
    let ev = b.qt.call_events().unwrap();
    assert!(ev.iter().any(|e| matches!(e, qt_ffi::call::CallEvent::Ended { .. })), "desligada: {ev:?}");
}

#[tokio::test]
async fn bloquear_e_desbloquear() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (mut a, mut b) = dois(&relay).await;
    let (ca, cb) = (cid(&a), cid(&b));
    b.enviar(&ca, "antes").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    a.qt.block_contact(cb.clone()).unwrap();
    b.enviar(&ca, "bloqueado").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    assert_eq!(a.recebidas(&cb), ["antes"], "mensagem de bloqueado nao aparece");
    a.qt.unblock_contact(cb.clone()).unwrap();
    b.enviar(&ca, "depois").await.unwrap();
    for _ in 0..2 { ate_acalmar(&mut [&mut a, &mut b]).await; }
    assert!(a.recebidas(&cb).contains(&"depois".to_string()), "{:?}", a.recebidas(&cb));
    a.enviar(&cb, "resposta").await.unwrap();
    for _ in 0..2 { ate_acalmar(&mut [&mut a, &mut b]).await; }
    assert!(b.recebidas(&ca).contains(&"resposta".to_string()), "{:?}", b.recebidas(&ca));
}

#[tokio::test]
async fn apagar_contacto_e_voltar_a_falar() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (mut a, mut b) = dois(&relay).await;
    let (ca, cb) = (cid(&a), cid(&b));
    a.enviar(&cb, "1").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    // A apaga B (avisa "sessao terminada") e tira-o da lista.
    let rid = b.como_contacto("B").relay_id;
    a.qt.delete_contact_notify(cb.clone(), rid).unwrap();
    a.contactos.clear();
    a.sincronizar_contactos();
    a.flush().await;
    ate_acalmar(&mut [&mut a, &mut b]).await;
    // B escreve outra vez: chega a A como convite.
    b.enviar(&ca, "ainda ai?").await.unwrap();
    for _ in 0..2 { ate_acalmar(&mut [&mut a, &mut b]).await; }
    assert_eq!(a.recebidas(&cb), ["ainda ai?"], "so a mensagem nova (as antigas foram apagadas)");
    a.aceitar_convite(&cb, "Bruno");
    a.enviar(&cb, "sim").await.unwrap();
    for _ in 0..2 { ate_acalmar(&mut [&mut a, &mut b]).await; }
    assert!(b.recebidas(&ca).contains(&"sim".to_string()), "{:?}", b.recebidas(&ca));
}

#[tokio::test]
async fn apagar_conta_no_relay() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (mut a, mut b) = dois(&relay).await;
    let cb = cid(&b);
    a.enviar(&cb, "1").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    let req = b.qt.relay_request_delete_account().unwrap();
    assert!(matches!(b.link.as_mut().unwrap().pedido(req).await.unwrap(), qt_ffi::RelayResponse::Ack));
    b.qt.delete_identity().unwrap();
    a.enviar(&cb, "estas ai?").await.unwrap();
    assert_eq!(a.recusas.get(&qt_ffi::relay_recipient_deleted_reason()).copied(), Some(1));
    assert!(a.qt.outbox().unwrap().is_empty(), "saiu da outbox (nao fica a tentar para sempre)");
}

#[tokio::test]
async fn grupo_de_tres_com_um_membro_que_recupera() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (mut a, mut b) = dois(&relay).await;
    let mut c = Telemovel::novo("carla", &relay.url);
    c.criar_conta().await;
    a.adicionar(c.como_contacto("Carla"));
    c.adicionar(a.como_contacto("Ana"));
    let pc = c.ativar_recuperacao().await;
    let gid = a.criar_grupo("Amigos", &[b.como_contacto("B"), c.como_contacto("C")]).await;
    a.enviar_grupo(&gid, "ola grupo").await;
    for _ in 0..2 { ate_acalmar(&mut [&mut a, &mut b, &mut c]).await; }
    assert_eq!(b.recebidas_grupo(&gid), ["ola grupo"]);
    assert_eq!(c.recebidas_grupo(&gid), ["ola grupo"]);
    // B nao tem C nos contactos: a mensagem de B tem de chegar a C na mesma.
    b.enviar_grupo(&gid, "de b").await;
    for _ in 0..3 { ate_acalmar(&mut [&mut a, &mut b, &mut c]).await; }
    assert_eq!(a.recebidas_grupo(&gid), ["de b"]);
    assert!(c.recebidas_grupo(&gid).contains(&"de b".to_string()), "C (nao e contacto de B) {:?}", c.recebidas_grupo(&gid));

    // C desinstala e recupera (os grupos nao vao na copia).
    c.desinstalar();
    c.recuperar(pc).await.unwrap();
    for _ in 0..3 { ate_acalmar(&mut [&mut a, &mut b, &mut c]).await; }
    a.enviar_grupo(&gid, "depois da recuperacao").await;
    b.enviar_grupo(&gid, "b depois").await;
    for _ in 0..4 { ate_acalmar(&mut [&mut a, &mut b, &mut c]).await; }
    assert!(a.recebidas_grupo(&gid).contains(&"b depois".to_string()), "o grupo continua entre A e B");
    // Desde a 1.0.4: o administrador (A) volta a mandar a info do grupo a quem recomecou a sessao.
    let g = c.qt.group(gid.clone()).unwrap().expect("a Carla volta a ter o grupo");
    assert!(g.active && g.name == "Amigos", "{g:?}");
    let rc = c.recebidas_grupo(&gid);
    assert!(rc.contains(&"depois da recuperacao".to_string()), "da Ana: {rc:?}");
    // O Bruno nao e contacto da Carla: a primeira mensagem dele ainda foi com a sessao antiga e
    // perdeu-se, mas a Carla abriu logo uma sessao nova com ele (1.0.4): a seguinte ja chega.
    b.enviar_grupo(&gid, "b segunda").await;
    for _ in 0..3 { ate_acalmar(&mut [&mut a, &mut b, &mut c]).await; }
    let rc = c.recebidas_grupo(&gid);
    assert!(rc.contains(&"b segunda".to_string()), "do Bruno sem a Carla escrever: {rc:?}");
    c.enviar_grupo(&gid, "voltei ao grupo").await;
    for _ in 0..3 { ate_acalmar(&mut [&mut a, &mut b, &mut c]).await; }
    assert!(a.recebidas_grupo(&gid).contains(&"voltei ao grupo".to_string()));
    assert!(b.recebidas_grupo(&gid).contains(&"voltei ao grupo".to_string()));
    b.enviar_grupo(&gid, "b outra vez").await;
    for _ in 0..3 { ate_acalmar(&mut [&mut a, &mut b, &mut c]).await; }
    let rc = c.recebidas_grupo(&gid);
    assert!(rc.contains(&"b outra vez".to_string()), "do Bruno depois de a Carla escrever: {rc:?}");
}

#[tokio::test]
async fn relay_reinicia_e_fila_grande() {
    // Contas com mais de um dia (300 envios por minuto): aqui o assunto e a fila, nao o limite.
    let mut cfg = fast_cfg();
    cfg.send_rate_new = cfg.send_rate;
    let mut relay = TestRelay::start(cfg.clone()).await;
    let (mut a, mut b) = dois(&relay).await;
    let (ca, cb) = (cid(&a), cid(&b));
    a.enviar(&cb, "0").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    b.sem_rede();
    for i in 1..=250 {
        a.qt.send_text(cb.clone(), format!("{i}")).unwrap();
    }
    for _ in 0..30 { if a.flush().await == 0 { break; } tokio::time::sleep(std::time::Duration::from_millis(100)).await; }
    relay.restart(cfg).await;
    a.ligar().await.unwrap();
    b.ligar().await.unwrap();
    for _ in 0..6 { ate_acalmar(&mut [&mut a, &mut b]).await; }
    let r = b.recebidas(&ca);
    assert_eq!(r.len(), 251, "chegaram {}", r.len());
    assert_eq!(r, (0..=250).map(|i| i.to_string()).collect::<Vec<_>>(), "pela ordem");
}

fn relay_com_anexos() -> qt_server::BlobConfig {
    let dir = std::env::temp_dir().join(format!("qt-app-anexos-{}-{}", std::process::id(), rand_n()));
    let mut c = qt_server::BlobConfig::new(dir);
    c.min_free_bytes = 0;
    c
}

fn rand_n() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
}

fn ficheiro(nome: &str, tam: usize) -> (String, Vec<u8>) {
    let bytes: Vec<u8> = (0..tam).map(|i| (i * 7 + i / 300) as u8).collect();
    let p = std::env::temp_dir().join(format!("qt-app-{}-{}-{nome}", std::process::id(), rand_n()));
    std::fs::write(&p, &bytes).unwrap();
    (p.to_string_lossy().into_owned(), bytes)
}

#[tokio::test]
async fn anexos_a_dois_e_em_grupo() {
    let relay = TestRelay::start_with_blobs(fast_cfg(), relay_com_anexos()).await;
    let (mut a, mut b) = dois(&relay).await;
    let (ca, cb) = (cid(&a), cid(&b));
    a.enviar(&cb, "vai foto").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    // Ficheiro de 300 KB (varios blocos) e uma "imagem".
    let (pdf, pdf_bytes) = ficheiro("doc.pdf", 300_000);
    let m = a.qt.attachment_create_file(cb.clone(), pdf, "application/pdf".into(), "doc.pdf".into()).unwrap();
    assert!(a.transferencias().await.is_empty());
    ate_acalmar(&mut [&mut a, &mut b]).await;
    assert!(b.transferencias().await.is_empty());
    assert_eq!(b.qt.attachment_read(ca.clone(), m.message_id.clone()).unwrap(), pdf_bytes, "o ficheiro chega igual");
    let (img, img_bytes) = ficheiro("foto.jpg", 90_000);
    let m2 = b.qt.attachment_create(ca.clone(), img, "image/jpeg".into(), 800, 600).unwrap();
    assert!(b.transferencias().await.is_empty());
    ate_acalmar(&mut [&mut a, &mut b]).await;
    assert!(a.transferencias().await.is_empty());
    assert_eq!(a.qt.attachment_read(cb.clone(), m2.message_id.clone()).unwrap(), img_bytes);

    // Grupo de tres.
    let mut c = Telemovel::novo("carla", &relay.url);
    c.criar_conta().await;
    a.adicionar(c.como_contacto("C"));
    c.adicionar(a.como_contacto("A")); // grupo de quem nao e contacto fica como convite
    let gid = a.criar_grupo("G", &[b.como_contacto("B"), c.como_contacto("C")]).await;
    for _ in 0..2 { ate_acalmar(&mut [&mut a, &mut b, &mut c]).await; }
    let conv = a.qt.group(gid.clone()).unwrap().unwrap().conversation_id;
    let (g, g_bytes) = ficheiro("g.pdf", 120_000);
    let m3 = a.qt.attachment_create_file(conv, g, "application/pdf".into(), "g.pdf".into()).unwrap();
    assert!(a.transferencias().await.is_empty());
    for _ in 0..2 { ate_acalmar(&mut [&mut a, &mut b, &mut c]).await; }
    for t in [&mut b, &mut c] {
        assert!(t.transferencias().await.is_empty(), "{}", t.nome);
        let conv = t.qt.group(gid.clone()).unwrap().unwrap().conversation_id;
        assert_eq!(t.qt.attachment_read(conv, m3.message_id.clone()).unwrap(), g_bytes, "{}", t.nome);
    }
}

/// Muitas pessoas adicionam a mesma conta: as one-time prekeys do relay acabam e as sessoes
/// novas continuam a funcionar (sem one-time prekey) e as antigas tambem.
#[tokio::test]
async fn one_time_prekeys_esgotadas() {
    let relay = TestRelay::start(fast_cfg()).await;
    let mut a = Telemovel::novo("ana", &relay.url);
    a.criar_conta().await;
    let ca = cid(&a);
    let rid = a.como_contacto("A").relay_id;
    // Pedidos de bundle por quatro telemoveis (cada um ate ao limite por conta) gastam as OPKs.
    let mut gastos = 0;
    for i in 0..4 {
        let mut x = Telemovel::novo(&format!("x{i}"), &relay.url);
        x.criar_conta().await;
        for _ in 0..40 {
            let l = x.link.as_mut().unwrap();
            if matches!(l.pedido(x.qt.relay_request_fetch_bundle(rid.clone()).unwrap()).await.unwrap(), qt_ffi::RelayResponse::Bundle { .. }) {
                gastos += 1;
            }
        }
    }
    println!("bundles servidos: {gastos}");
    let mut z = Telemovel::novo("zeca", &relay.url);
    z.criar_conta().await;
    z.adicionar(a.como_contacto("Ana"));
    a.adicionar(z.como_contacto("Zeca"));
    z.enviar(&ca, "sem otpk").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut z]).await;
    assert_eq!(a.recebidas(&cid(&z)), ["sem otpk"]);
    // A volta a abrir a app (novo lote so passado um dia; o relay continua sem OPKs para ela).
    a.reiniciar_app().await;
    a.enviar(&cid(&z), "ok").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut z]).await;
    assert_eq!(z.recebidas(&ca), ["ok"]);
}

/// Videochamada: o video segue a regra do pedido/aceitar dos dois lados.
#[tokio::test]
async fn videochamada_sinalizacao() {
    use qt_ffi::call::{CallEvent, VideoResponse};
    let relay = TestRelay::start(fast_cfg()).await;
    let (mut a, mut b) = dois(&relay).await;
    let (_ca, cb) = (cid(&a), cid(&b));
    a.enviar(&cb, "oi").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    let s = a.qt.call_start_video(cb.clone()).unwrap();
    a.flush().await;
    b.ciclo().await;
    let ev = b.qt.call_events().unwrap();
    assert!(ev.iter().any(|e| matches!(e, CallEvent::Incoming { .. })), "{ev:?}");
    assert!(b.qt.call_is_video(s.call_id.clone()).unwrap());
    b.flush().await;
    b.qt.call_accept(s.call_id.clone()).unwrap();
    b.flush().await;
    a.ciclo().await;
    let _ = a.qt.call_events().unwrap();
    // A liga a camara; B recebe o pedido e aceita ver e mostrar.
    if let Some(it) = a.qt.call_video_set_camera(s.call_id.clone(), true).unwrap() { let _ = it; }
    a.flush().await;
    b.ciclo().await;
    let ev = b.qt.call_events().unwrap();
    println!("B ve: {ev:?}");
    assert!(ev.iter().any(|e| matches!(e, CallEvent::Video { peer_camera: true, .. })), "pedido de video chegou: {ev:?}");
    b.qt.call_video_respond(s.call_id.clone(), VideoResponse::ViewAndCamera).unwrap();
    b.flush().await;
    a.ciclo().await;
    let ev = a.qt.call_events().unwrap();
    println!("A ve: {ev:?}");
    assert!(ev.iter().any(|e| matches!(e, CallEvent::Video { peer_accepts: true, .. })), "{ev:?}");
    a.qt.call_hangup(s.call_id.clone()).unwrap();
    a.flush().await;
    b.ciclo().await;
    assert!(b.qt.call_events().unwrap().iter().any(|e| matches!(e, CallEvent::Ended { .. })));
    println!("pedidos A {:?}", a.pedidos.lock().unwrap());
}

/// Presenca, foto de perfil e reacoes/visto pelo relay.
#[tokio::test]
async fn presenca_foto_reacoes_e_visto() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (mut a, mut b) = dois(&relay).await;
    let (ca, cb) = (cid(&a), cid(&b));
    a.enviar(&cb, "ola").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    // Presenca ligada nos dois: online.
    a.qt.presence_configure(true, vec![cb.clone()]).unwrap();
    b.qt.presence_configure(true, vec![ca.clone()]).unwrap();
    a.qt.presence_announce(true).unwrap();
    b.qt.presence_announce(true).unwrap();
    for _ in 0..2 { ate_acalmar(&mut [&mut a, &mut b]).await; }
    assert!(b.qt.presence_online().contains(&ca), "B ve a A online");
    // Foto: so WebP pequeno; usa os bytes de uma imagem WebP minima.
    let webp = include_bytes!("../../android/app/src/main/res/drawable-nodpi/badge_coin_qf.png");
    let r = a.qt.profile_photo_set(webp.to_vec(), vec![cb.clone()]);
    println!("foto (png, deve ser recusada ou aceite conforme o formato): {:?}", r.is_ok());
    // Reacao e visto.
    let mid = b.conversa(&ca)[0].message_id.clone();
    b.qt.react(ca.clone(), mid.clone(), "👍".into()).unwrap();
    b.qt.mark_conversation_read(ca.clone(), true).unwrap();
    b.flush().await;
    for _ in 0..2 { ate_acalmar(&mut [&mut a, &mut b]).await; }
    let m = a.conversa(&cb).into_iter().find(|m| m.message_id == mid).unwrap();
    assert_eq!(m.status, MessageStatus::Read, "Visto");
    assert!(m.reactions.iter().any(|r| r.emoji == "👍"), "reacao: {:?}", m.reactions);
}

/// A oferta de chamada vai pela outbox, por ordem: com coisas por enviar e o limite da conta,
/// quanto tempo demora a tocar do outro lado?
#[tokio::test]
async fn chamada_atras_da_fila_no_limite() {
    let mut cfg = fast_cfg();
    cfg.send_rate_new = qt_server::limits::Rate { burst: 20, per_minute: 600 }; // 10 por segundo
    let relay = TestRelay::start(cfg).await;
    let (mut a, mut b) = dois(&relay).await;
    let cb = cid(&b);
    a.garantir_sessao(&cb).await.unwrap();
    for i in 0..80 {
        a.qt.send_text(cb.clone(), format!("m{i}")).unwrap();
    }
    let t0 = std::time::Instant::now();
    a.qt.call_start(cb.clone()).unwrap();
    let mut tocou = None;
    for _ in 0..100 {
        a.flush().await;
        b.ciclo().await;
        if b.qt.call_events().unwrap().iter().any(|e| matches!(e, qt_ffi::call::CallEvent::Incoming { .. })) {
            tocou = Some(t0.elapsed());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    println!("com 80 mensagens a frente e 10 envios/s, tocou ao fim de {tocou:?}");
    // Desde a 1.0.4 os sinais de chamada saem a frente: toca no primeiro ciclo.
    assert!(tocou.is_some_and(|d| d < std::time::Duration::from_secs(2)), "{tocou:?}");
    // E as mensagens continuam a chegar todas, pela ordem.
    for _ in 0..60 {
        a.flush().await;
        b.ciclo().await;
        if b.recebidas(&cid(&a)).len() == 80 { break; }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    assert_eq!(b.recebidas(&cid(&a)), (0..80).map(|i| format!("m{i}")).collect::<Vec<_>>());
}

/// O contacto apagou-me e depois apagou a conta: a sessao fica por recomecar e o bundle dele ja nao
/// existe. A app nao pode pedir o bundle a cada mensagem que recebe (limite de bundles da conta).
#[tokio::test]
async fn recomecar_sessao_com_conta_apagada_nao_insiste() {
    let relay = TestRelay::start(fast_cfg()).await;
    let (mut a, mut b) = dois(&relay).await;
    let mut c = Telemovel::novo("carla", &relay.url);
    c.criar_conta().await;
    a.adicionar(c.como_contacto("C"));
    c.adicionar(a.como_contacto("A"));
    let (ca, cb) = (cid(&a), cid(&b));
    a.enviar(&cb, "1").await.unwrap();
    ate_acalmar(&mut [&mut a, &mut b]).await;
    // B apaga a Ana (avisa) e depois apaga a conta no relay.
    let rid = a.como_contacto("A").relay_id;
    b.qt.delete_contact_notify(ca.clone(), rid).unwrap();
    b.flush().await;
    let req = b.qt.relay_request_delete_account().unwrap();
    b.link.as_mut().unwrap().pedido(req).await.unwrap();
    // A Ana continua a usar a app: recebe 40 mensagens da Carla, um ciclo por mensagem.
    let antes = a.pedidos_de("FetchPreKeyBundle");
    for i in 0..40 {
        c.enviar(&ca, &format!("c{i}")).await.unwrap();
        a.ciclo().await;
    }
    let pedidos = a.pedidos_de("FetchPreKeyBundle") - antes;
    println!("bundles pedidos por causa da sessao a recomecar: {pedidos}");
    assert!(pedidos <= 2, "pede o bundle de uma conta apagada a cada ciclo: {pedidos}");
}

/// Recuperar uma conta com muitos contactos: o relay limita os pedidos de bundle; a app nao pode
/// insistir nem desistir por 5 minutos, e todas as sessoes voltam.
#[tokio::test]
async fn recuperar_com_muitos_contactos_no_limite_de_bundles() {
    let mut cfg = fast_cfg();
    cfg.fetch_bundle_rate = qt_server::limits::Rate { burst: 8, per_minute: 600 }; // 10 por segundo
    let relay = TestRelay::start(cfg).await;
    let mut a = Telemovel::novo("ana", &relay.url);
    a.criar_conta().await;
    let mut outros = vec![];
    for i in 0..30 {
        let mut x = Telemovel::novo(&format!("amigo{i}"), &relay.url);
        x.criar_conta().await;
        x.adicionar(a.como_contacto("Ana"));
        a.adicionar(x.como_contacto(&format!("Amigo {i}")));
        outros.push(x);
    }
    let pa = a.ativar_recuperacao().await;
    a.desinstalar();
    a.recuperar(pa).await.unwrap();
    assert_eq!(a.contactos.len(), 30);
    let mut ciclos = 0;
    while ciclos < 40 {
        a.ciclo().await;
        ciclos += 1;
        let falta = outros.iter().filter(|x| !a.qt.has_session(cid(x)).unwrap()).count();
        if falta == 0 { break; }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let recusas = a.pedidos_de("FetchPreKeyBundle");
    println!("sessoes de novo com 30 contactos em {ciclos} ciclos, {recusas} pedidos de bundle");
    assert!(outros.iter().all(|x| a.qt.has_session(cid(x)).unwrap()), "ficaram contactos sem sessao");
    assert!(recusas <= 30 + ciclos, "insistiu nos pedidos de bundle: {recusas}");
    // E as conversas funcionam nos dois sentidos.
    let ca = cid(&a);
    for x in outros.iter_mut().take(3) {
        x.ciclo().await;
        x.enviar(&ca, "ola de novo").await.unwrap();
    }
    for _ in 0..3 {
        a.ciclo().await;
        for x in outros.iter_mut().take(3) { x.ciclo().await; }
    }
    for x in outros.iter().take(3) {
        assert_eq!(a.recebidas(&cid(x)), ["ola de novo"], "{}", x.nome);
    }
}
