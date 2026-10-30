//! Comunidade (0.9.52): a lista das pessoas com medalha, so com a foto e a medalha (sem nome nem ID).
//!
//! Tres pecas, todas aqui para as apps e a ferramenta do operador usarem o mesmo codigo:
//!
//!   * ENVIO DA FOTO (app de quem tem medalha): a miniatura que ja vai para os contactos, o
//!     certificado da medalha e a hora, autenticados com a chave de IDENTIDADE da conta e
//!     cifrados para a chave do operador com o metodo das denuncias (`report::seal_with`). O
//!     relay guarda bytes que nao consegue ler.
//!     A autenticacao e um HMAC com a chave que sai de DH(identidade X25519 da conta, chave
//!     publica das denuncias). So quem tem a chave privada da identidade do certificado a
//!     consegue fazer, e so o operador (com a semente) a confere. Assim o certificado sozinho
//!     (que os contactos recebem) nao chega para por a foto de outra pessoa na lista.
//!   * INDICE (feito pelo operador): numero de versao, data e, por membro, so a medalha e o
//!     hash da foto, ja pela ordem da lista (medalha e, dentro de cada uma, ordem de entrada). Assinado com a chave das MEDALHAS (a do emissor, `badge::ISSUER_PUBLIC_KEY`)
//!     e cifrado com [`COMMUNITY_KEY`]. Nunca leva IDs nem certificados.
//!   * FOTOS: cada uma cifrada a parte com [`COMMUNITY_KEY`] e identificada pelo SHA-256 dos
//!     bytes cifrados (o indice assinado tem o hash, por isso uma foto trocada e recusada).
//!
//! [`COMMUNITY_KEY`] vai dentro das apps: protege a lista no armazenamento da AWS e de quem nao
//! tem o Sealer, mas nao de quem instala a app. Isso esta aceite (docs/threat-model.md); o que
//! a lista mostra e o que qualquer pessoa com a app ve.
//!
//! FORMATOS:
//!   * Envio (antes de selar): `[MAGIC][106 certificado][8 enviada_em ms BE][1 tem_foto]
//!     [foto][32 HMAC]`. O HMAC cobre tudo o que vem antes.
//!   * Indice (claro): `[1 versao=1][8 numero BE][8 criado_em ms BE][2 membros BE]` e por
//!     membro `[1 medalha][1 tem_foto][32 hash se tem_foto]`;
//!     segue-se a assinatura Ed25519 (64) sobre `INDEX_DOMAIN || claro`.
//!     Cifrado: `[1 versao=1][12 nonce][ChaCha20-Poly1305]`, AAD `INDEX_AAD`.
//!   * Foto: `[1 versao=1][12 nonce][ChaCha20-Poly1305 da foto]`, AAD `PHOTO_AAD`.

use crate::{badge, report};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

/// Chave da lista (indice e fotos), igual em todas as apps e na ferramenta do operador.
pub const COMMUNITY_KEY: [u8; 32] = [0x60, 0xb0, 0x8e, 0x49, 0xaa, 0xd9, 0x16, 0xfe, 0x80, 0x55, 0x4c, 0x3d, 0x02, 0x59, 0xfe, 0x25, 0xdf, 0xdb, 0x79, 0x3a, 0x3c, 0x73, 0xed, 0xcc, 0xd2, 0xa2, 0x5a, 0x98, 0x4c, 0x2b, 0x80, 0x3a];

pub const VERSION: u8 = 1;
const SUB_MAGIC: &[u8] = b"SEALER-COMMUNITY-SUB-v1";
const AUTH_SALT: &[u8] = b"SEALER-COMMUNITY";
const AUTH_INFO: &[u8] = b"SEALER-COMMUNITY-AUTH-v1";
const INDEX_DOMAIN: &[u8] = b"SEALER-COMMUNITY-INDEX-v1";
const INDEX_AAD: &[u8] = b"SEALER-COMMUNITY-LIST-v1";
const PHOTO_AAD: &[u8] = b"SEALER-COMMUNITY-PHOTO-v1";

/// Maior foto aceite (a miniatura dos contactos tem no maximo 9 973 bytes).
pub const MAX_PHOTO: usize = 10_000;
/// Foto cifrada, no maximo.
pub const MAX_PHOTO_BLOB: usize = 1 + 12 + MAX_PHOTO + 16;
/// Membros por lista. O indice cifrado fica abaixo de [`MAX_INDEX_BLOB`], que cabe numa
/// mensagem do relay (Noise: 65 535 bytes, em base64).
pub const MAX_MEMBERS: usize = 1_000;
const INDEX_HEADER: usize = 1 + 8 + 8 + 2;
const MEMBER_MAX: usize = 1 + 1 + 32;
/// Indice cifrado, no maximo.
pub const MAX_INDEX_BLOB: usize = 1 + 12 + INDEX_HEADER + MAX_MEMBERS * MEMBER_MAX + 64 + 16;
/// Envio selado, no maximo (o limite das denuncias).
pub const MAX_SUBMISSION: usize = report::MAX_SEALED;

#[derive(Debug, PartialEq, Eq)]
pub enum CommunityError {
    /// Chave das denuncias por preencher (zeros): a app nao envia.
    NoOperatorKey,
    BadFormat,
    /// Certificado invalido (assinatura, formato) ou sem medalha.
    BadCertificate,
    /// A autenticacao nao e da identidade do certificado.
    BadAuth,
    /// Assinatura do indice invalida.
    BadSignature,
    TooLarge,
}

/// O que o operador tira de um envio valido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submission {
    pub cert: badge::BadgeCert,
    /// O certificado tal como veio (106 bytes).
    pub cert_bytes: Vec<u8>,
    pub sent_at_ms: i64,
    pub photo: Option<Vec<u8>>,
}

/// Um membro da lista: so a medalha e a foto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub badge: u8,
    /// SHA-256 da foto cifrada; `None` = sem foto (as apps mostram a inicial).
    pub photo_hash: Option<[u8; 32]>,
}

/// A lista (indice), ja verificada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    pub version: u64,
    pub created_ms: u64,
    pub members: Vec<Member>,
}

/// WebP ou JPEG, dentro do tamanho (a mesma regra da foto de perfil).
pub fn valid_photo(p: &[u8]) -> bool {
    let webp = p.len() >= 12 && &p[..4] == b"RIFF" && &p[8..12] == b"WEBP";
    let jpeg = p.len() >= 3 && p[..3] == [0xFF, 0xD8, 0xFF];
    (webp || jpeg) && p.len() <= MAX_PHOTO
}

// ------------------------------------------------------------------ envio da foto

fn auth_key(shared: &[u8; 32], subject: &[u8; 32], op_pub: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(Some(AUTH_SALT), shared);
    let mut k = Zeroizing::new([0u8; 32]);
    let info = [AUTH_INFO, subject.as_slice(), op_pub.as_slice()].concat();
    hk.expand(&info, k.as_mut()).expect("32 bytes cabem no HKDF");
    k
}

fn mac(key: &[u8; 32], body: &[u8]) -> Hmac<Sha256> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC aceita 32 bytes");
    m.update(body);
    m
}

/// Conteudo do envio (antes de selar), autenticado com a identidade `identity` (a chave
/// privada X25519 da conta, a do certificado). `op_pub`: a chave publica das denuncias.
pub fn submission_content(
    op_pub: &[u8; 32],
    identity: &StaticSecret,
    cert: &[u8],
    sent_at_ms: i64,
    photo: Option<&[u8]>,
) -> Result<Vec<u8>, CommunityError> {
    if cert.len() != badge::CERT_LEN {
        return Err(CommunityError::BadCertificate);
    }
    if photo.is_some_and(|p| !valid_photo(p)) {
        return Err(CommunityError::BadFormat);
    }
    let subject = PublicKey::from(identity).to_bytes();
    if cert[10..42] != subject {
        return Err(CommunityError::BadCertificate);
    }
    let mut body = Vec::with_capacity(SUB_MAGIC.len() + cert.len() + 9 + photo.map_or(0, <[u8]>::len) + 32);
    body.extend_from_slice(SUB_MAGIC);
    body.extend_from_slice(cert);
    body.extend_from_slice(&sent_at_ms.to_be_bytes());
    match photo {
        Some(p) => {
            body.push(1);
            body.extend_from_slice(p);
        }
        None => body.push(0),
    }
    let shared = Zeroizing::new(identity.diffie_hellman(&PublicKey::from(*op_pub)).to_bytes());
    let key = auth_key(&shared, &subject, op_pub);
    let tag = mac(&key, &body).finalize().into_bytes();
    body.extend_from_slice(&tag);
    Ok(body)
}

/// O envio pronto para o relay: o conteudo selado para a chave das denuncias.
pub fn seal_submission_with(
    op_pub: &[u8; 32],
    identity: &StaticSecret,
    cert: &[u8],
    sent_at_ms: i64,
    photo: Option<&[u8]>,
) -> Result<Vec<u8>, CommunityError> {
    if op_pub == &[0u8; 32] {
        return Err(CommunityError::NoOperatorKey);
    }
    let content = submission_content(op_pub, identity, cert, sent_at_ms, photo)?;
    report::seal_with(op_pub, &content).map_err(|e| match e {
        report::ReportError::NoOperatorKey => CommunityError::NoOperatorKey,
        report::ReportError::TooLarge => CommunityError::TooLarge,
        report::ReportError::BadFormat => CommunityError::BadFormat,
    })
}

/// Com a chave fixa das denuncias.
pub fn seal_submission(
    identity: &StaticSecret,
    cert: &[u8],
    sent_at_ms: i64,
    photo: Option<&[u8]>,
) -> Result<Vec<u8>, CommunityError> {
    seal_submission_with(&report::OPERATOR_REPORT_PUBLIC, identity, cert, sent_at_ms, photo)
}

/// So o operador: abre um envio com a semente do operador e confere o certificado (com a chave
/// do emissor `issuer`) e a autenticacao da identidade. Nao sabe se a medalha foi retirada
/// depois: isso e o registo do emissor (ferramenta `comunidade`).
pub fn open_submission(seed: &[u8; 32], issuer: &[u8; 32], sealed: &[u8]) -> Result<Submission, CommunityError> {
    let body = Zeroizing::new(report::open(seed, sealed).map_err(|_| CommunityError::BadFormat)?);
    let min = SUB_MAGIC.len() + badge::CERT_LEN + 8 + 1 + 32;
    if body.len() < min || !body.starts_with(SUB_MAGIC) {
        return Err(CommunityError::BadFormat);
    }
    let (signed, tag) = body.split_at(body.len() - 32);
    let mut at = SUB_MAGIC.len();
    let cert_bytes = signed[at..at + badge::CERT_LEN].to_vec();
    at += badge::CERT_LEN;
    let cert = badge::verify_with(issuer, &cert_bytes).ok_or(CommunityError::BadCertificate)?;
    if cert.badge == badge::BADGE_NONE {
        return Err(CommunityError::BadCertificate);
    }
    let op_secret = report::operator_report_secret(seed);
    let op_pub = PublicKey::from(&op_secret).to_bytes();
    let shared = Zeroizing::new(op_secret.diffie_hellman(&PublicKey::from(cert.subject)).to_bytes());
    let key = auth_key(&shared, &cert.subject, &op_pub);
    mac(&key, signed).verify_slice(tag).map_err(|_| CommunityError::BadAuth)?;
    let sent_at_ms = i64::from_be_bytes(signed[at..at + 8].try_into().map_err(|_| CommunityError::BadFormat)?);
    at += 8;
    let photo = match (signed[at], &signed[at + 1..]) {
        (0, []) => None,
        (1, p) if valid_photo(p) => Some(p.to_vec()),
        _ => return Err(CommunityError::BadFormat),
    };
    Ok(Submission { cert, cert_bytes, sent_at_ms, photo })
}

// ------------------------------------------------------------------ indice e fotos

fn aead_seal(key: &[u8; 32], plain: &[u8], aad: &[u8]) -> Vec<u8> {
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let ct = ChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plain, aad })
        .expect("cifrar em memoria nao falha");
    let mut out = Vec::with_capacity(1 + 12 + ct.len());
    out.push(VERSION);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

fn aead_open(key: &[u8; 32], blob: &[u8], aad: &[u8]) -> Result<Vec<u8>, CommunityError> {
    if blob.len() < 1 + 12 + 16 || blob[0] != VERSION {
        return Err(CommunityError::BadFormat);
    }
    ChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(Nonce::from_slice(&blob[1..13]), Payload { msg: &blob[13..], aad })
        .map_err(|_| CommunityError::BadFormat)
}

/// Ordem da lista: por medalha (Founder, Co-founder, Dev, ... como os numeros). Dentro de cada
/// medalha fica a ordem em que vieram, que e a de entrada (o operador monta o indice assim).
pub fn sort_members(m: &mut [Member]) {
    m.sort_by_key(|x| x.badge); // estavel: mantem a ordem de entrada
}

fn encode_index(ix: &Index) -> Result<Vec<u8>, CommunityError> {
    if ix.members.len() > MAX_MEMBERS {
        return Err(CommunityError::TooLarge);
    }
    let mut out = Vec::with_capacity(INDEX_HEADER + ix.members.len() * MEMBER_MAX);
    out.push(VERSION);
    out.extend_from_slice(&ix.version.to_be_bytes());
    out.extend_from_slice(&ix.created_ms.to_be_bytes());
    out.extend_from_slice(&(ix.members.len() as u16).to_be_bytes());
    for m in &ix.members {
        if m.badge == badge::BADGE_NONE || m.badge > badge::BADGE_MAX {
            return Err(CommunityError::BadCertificate);
        }
        out.push(m.badge);
        match &m.photo_hash {
            Some(h) => {
                out.push(1);
                out.extend_from_slice(h);
            }
            None => out.push(0),
        }
    }
    Ok(out)
}

fn decode_index(b: &[u8]) -> Result<Index, CommunityError> {
    let bad = || CommunityError::BadFormat;
    if b.len() < INDEX_HEADER || b[0] != VERSION {
        return Err(bad());
    }
    let version = u64::from_be_bytes(b[1..9].try_into().map_err(|_| bad())?);
    let created_ms = u64::from_be_bytes(b[9..17].try_into().map_err(|_| bad())?);
    let n = u16::from_be_bytes(b[17..19].try_into().map_err(|_| bad())?) as usize;
    if n > MAX_MEMBERS {
        return Err(bad());
    }
    let mut at = INDEX_HEADER;
    let mut members = Vec::with_capacity(n);
    let take = |at: &mut usize, len: usize| -> Result<&[u8], CommunityError> {
        let s = b.get(*at..*at + len).ok_or(CommunityError::BadFormat)?;
        *at += len;
        Ok(s)
    };
    for _ in 0..n {
        let badge = take(&mut at, 1)?[0];
        if badge == badge::BADGE_NONE || badge > badge::BADGE_MAX {
            return Err(bad());
        }
        let photo_hash = match take(&mut at, 1)?[0] {
            0 => None,
            1 => Some(take(&mut at, 32)?.try_into().map_err(|_| bad())?),
            _ => return Err(bad()),
        };
        members.push(Member { badge, photo_hash });
    }
    if at != b.len() {
        return Err(bad());
    }
    Ok(Index { version, created_ms, members })
}

/// So o operador: assina o indice com a chave das medalhas (`issuer_seed`) e cifra-o com `key`.
pub fn build_index_with(issuer_seed: &[u8; 32], key: &[u8; 32], ix: &Index) -> Result<Vec<u8>, CommunityError> {
    let mut plain = encode_index(ix)?;
    let sig = SigningKey::from_bytes(issuer_seed).sign(&[INDEX_DOMAIN, &plain].concat());
    plain.extend_from_slice(&sig.to_bytes());
    let out = aead_seal(key, &plain, INDEX_AAD);
    if out.len() > MAX_INDEX_BLOB {
        return Err(CommunityError::TooLarge);
    }
    Ok(out)
}

/// Com a chave da lista das apps.
pub fn build_index(issuer_seed: &[u8; 32], ix: &Index) -> Result<Vec<u8>, CommunityError> {
    build_index_with(issuer_seed, &COMMUNITY_KEY, ix)
}

/// Decifra e confere a assinatura do emissor `issuer`.
pub fn open_index_with(issuer: &[u8; 32], key: &[u8; 32], blob: &[u8]) -> Result<Index, CommunityError> {
    if blob.len() > MAX_INDEX_BLOB {
        return Err(CommunityError::TooLarge);
    }
    let plain = aead_open(key, blob, INDEX_AAD)?;
    if plain.len() < INDEX_HEADER + 64 || issuer == &[0u8; 32] {
        return Err(CommunityError::BadFormat);
    }
    let (body, sig) = plain.split_at(plain.len() - 64);
    let vk = VerifyingKey::from_bytes(issuer).map_err(|_| CommunityError::BadSignature)?;
    let sig = Signature::from_slice(sig).map_err(|_| CommunityError::BadSignature)?;
    vk.verify_strict(&[INDEX_DOMAIN, body].concat(), &sig).map_err(|_| CommunityError::BadSignature)?;
    decode_index(body)
}

/// Com a chave do emissor e a chave da lista das apps.
pub fn open_index(blob: &[u8]) -> Result<Index, CommunityError> {
    open_index_with(&badge::ISSUER_PUBLIC_KEY, &COMMUNITY_KEY, blob)
}

/// Hash de uma foto cifrada (o que vai no indice e identifica a foto no relay).
pub fn photo_hash(blob: &[u8]) -> [u8; 32] {
    Sha256::digest(blob).into()
}

/// So o operador: cifra uma foto. Devolve (hash, foto cifrada).
pub fn seal_photo_with(key: &[u8; 32], photo: &[u8]) -> Result<([u8; 32], Vec<u8>), CommunityError> {
    if !valid_photo(photo) {
        return Err(CommunityError::BadFormat);
    }
    let blob = aead_seal(key, photo, PHOTO_AAD);
    Ok((photo_hash(&blob), blob))
}

pub fn seal_photo(photo: &[u8]) -> Result<([u8; 32], Vec<u8>), CommunityError> {
    seal_photo_with(&COMMUNITY_KEY, photo)
}

/// Confere que a foto cifrada e a do `hash` (do indice assinado) e decifra-a.
pub fn open_photo_with(key: &[u8; 32], hash: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, CommunityError> {
    if blob.len() > MAX_PHOTO_BLOB || &photo_hash(blob) != hash {
        return Err(CommunityError::BadFormat);
    }
    let p = aead_open(key, blob, PHOTO_AAD)?;
    if !valid_photo(&p) {
        return Err(CommunityError::BadFormat);
    }
    Ok(p)
}

pub fn open_photo(hash: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, CommunityError> {
    open_photo_with(&COMMUNITY_KEY, hash, blob)
}

// ------------------------------------------------------------------ estado do operador

const STATE_AAD: &[u8] = b"SEALER-COMMUNITY-STATE-v1";

/// So o operador: chave do ficheiro de estado da Comunidade (os membros, as fotos e o que ja
/// foi tratado), derivada da semente do operador. O ficheiro fica no PC dele, nunca na AWS.
pub fn operator_state_key(seed: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(Some(b"SEALER-OPERATOR"), seed);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(b"community-state", out.as_mut()).expect("32 bytes cabem no HKDF");
    out
}

pub fn seal_state(seed: &[u8; 32], plain: &[u8]) -> Vec<u8> {
    aead_seal(&operator_state_key(seed), plain, STATE_AAD)
}

pub fn open_state(seed: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, CommunityError> {
    aead_open(&operator_state_key(seed), blob, STATE_AAD)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn webp(n: usize) -> Vec<u8> {
        let mut p = [b"RIFF".as_slice(), &[0, 0, 0, 0], b"WEBPVP8 "].concat();
        p.resize(n, 7);
        p
    }

    struct Setup {
        op_seed: [u8; 32],
        op_pub: [u8; 32],
        issuer_seed: [u8; 32],
        issuer: [u8; 32],
        me: StaticSecret,
        cert: Vec<u8>,
    }

    fn setup(badge_num: u8) -> Setup {
        let op_seed = [3u8; 32];
        let issuer_seed = [4u8; 32];
        let me = StaticSecret::from([5u8; 32]);
        let subject = PublicKey::from(&me).to_bytes();
        Setup {
            op_seed,
            op_pub: report::operator_report_public(&op_seed),
            issuer_seed,
            issuer: badge::issuer_public(&issuer_seed),
            cert: badge::issue(&issuer_seed, &subject, badge_num, 1_700_000_000_000),
            me,
        }
    }

    #[test]
    fn envio_valido_abre_no_operador() {
        let s = setup(badge::BADGE_DEV);
        let photo = webp(9_973);
        let sealed = seal_submission_with(&s.op_pub, &s.me, &s.cert, 42, Some(&photo)).unwrap();
        assert!(sealed.len() <= MAX_SUBMISSION);
        let got = open_submission(&s.op_seed, &s.issuer, &sealed).unwrap();
        assert_eq!(got.cert.badge, badge::BADGE_DEV);
        assert_eq!(got.cert_bytes, s.cert);
        assert_eq!((got.sent_at_ms, got.photo), (42, Some(photo)));
        // "Sem foto" tambem e um envio valido.
        let sealed = seal_submission_with(&s.op_pub, &s.me, &s.cert, 43, None).unwrap();
        assert_eq!(open_submission(&s.op_seed, &s.issuer, &sealed).unwrap().photo, None);
    }

    #[test]
    fn certificado_de_outra_pessoa_e_recusado() {
        let victim = setup(badge::BADGE_FOUNDER);
        // O atacante tem o certificado da vitima (os contactos recebem-no), mas nao a chave dela.
        let attacker = StaticSecret::from([9u8; 32]);
        assert_eq!(
            seal_submission_with(&victim.op_pub, &attacker, &victim.cert, 1, None),
            Err(CommunityError::BadCertificate)
        );
        // Montado a mao, sem passar pela verificacao da app: a autenticacao nao bate certo.
        let mut body = [SUB_MAGIC, &victim.cert, &1i64.to_be_bytes(), &[0u8]].concat();
        let shared = attacker.diffie_hellman(&PublicKey::from(victim.op_pub)).to_bytes();
        let subject: [u8; 32] = victim.cert[10..42].try_into().unwrap();
        let tag = mac(&auth_key(&shared, &subject, &victim.op_pub), &body).finalize().into_bytes();
        body.extend_from_slice(&tag);
        let sealed = report::seal_with(&victim.op_pub, &body).unwrap();
        assert_eq!(open_submission(&victim.op_seed, &victim.issuer, &sealed), Err(CommunityError::BadAuth));
    }

    #[test]
    fn envios_estragados_ou_sem_medalha() {
        let s = setup(badge::BADGE_TESTER);
        // Outro emissor: certificado recusado.
        let sealed = seal_submission_with(&s.op_pub, &s.me, &s.cert, 1, None).unwrap();
        assert_eq!(
            open_submission(&s.op_seed, &badge::issuer_public(&[1u8; 32]), &sealed),
            Err(CommunityError::BadCertificate)
        );
        // Medalha retirada (badge 0): recusado.
        let none = setup(badge::BADGE_NONE);
        let sealed = seal_submission_with(&none.op_pub, &none.me, &none.cert, 1, None).unwrap();
        assert_eq!(open_submission(&none.op_seed, &none.issuer, &sealed), Err(CommunityError::BadCertificate));
        // Foto que nao e WebP/JPEG ou grande demais.
        assert_eq!(seal_submission_with(&s.op_pub, &s.me, &s.cert, 1, Some(b"GIF89a")), Err(CommunityError::BadFormat));
        assert_eq!(
            seal_submission_with(&s.op_pub, &s.me, &s.cert, 1, Some(&webp(MAX_PHOTO + 1))),
            Err(CommunityError::BadFormat)
        );
        // Chave do operador por preencher.
        assert_eq!(seal_submission_with(&[0u8; 32], &s.me, &s.cert, 1, None), Err(CommunityError::NoOperatorKey));
        // Outra semente do operador nao abre.
        assert_eq!(open_submission(&[8u8; 32], &s.issuer, &sealed), Err(CommunityError::BadFormat));
    }

    fn sample(n: usize) -> Index {
        Index {
            version: 1_730_000_000,
            created_ms: 1_730_000_000_000,
            members: (0..n)
                .map(|i| Member {
                    badge: (i % 9) as u8 + 1,
                    photo_hash: (i % 2 == 0).then_some([i as u8; 32]),
                })
                .collect(),
        }
    }

    #[test]
    fn indice_assinado_e_cifrado() {
        let s = setup(badge::BADGE_DEV);
        let key = [6u8; 32];
        let ix = sample(5);
        let blob = build_index_with(&s.issuer_seed, &key, &ix).unwrap();
        assert_eq!(open_index_with(&s.issuer, &key, &blob).unwrap(), ix);
        // Outra chave da lista, outro emissor ou um byte mexido: recusado.
        assert!(open_index_with(&s.issuer, &[7u8; 32], &blob).is_err());
        assert_eq!(
            open_index_with(&badge::issuer_public(&[1u8; 32]), &key, &blob),
            Err(CommunityError::BadSignature)
        );
        let mut m = blob.clone();
        m[20] ^= 1;
        assert!(open_index_with(&s.issuer, &key, &m).is_err());
        // Lista vazia (ninguem com nome ainda) tambem serve.
        let empty = Index { members: vec![], ..ix };
        let blob = build_index_with(&s.issuer_seed, &key, &empty).unwrap();
        assert_eq!(open_index_with(&s.issuer, &key, &blob).unwrap(), empty);
    }

    #[test]
    fn limites_do_indice() {
        let s = setup(badge::BADGE_DEV);
        let mut ix = sample(MAX_MEMBERS);
        for m in &mut ix.members {
            m.photo_hash = Some([1u8; 32]);
        }
        let blob = build_index(&s.issuer_seed, &ix).unwrap();
        assert!(blob.len() <= MAX_INDEX_BLOB);
        // Em base64 cabe numa mensagem do relay (com folga para o JSON).
        assert!(blob.len().div_ceil(3) * 4 < 62_000);
        ix.members.push(ix.members[0].clone());
        assert_eq!(build_index(&s.issuer_seed, &ix), Err(CommunityError::TooLarge));
        // Medalha 0 ou desconhecida: recusada.
        for b in [badge::BADGE_NONE, badge::BADGE_MAX + 1] {
            let ix = Index { members: vec![Member { badge: b, photo_hash: None }], ..sample(0) };
            assert_eq!(build_index(&s.issuer_seed, &ix), Err(CommunityError::BadCertificate));
        }
    }

    #[test]
    fn fotos_identificadas_pelo_hash() {
        let key = [6u8; 32];
        let photo = webp(5_000);
        let (h, blob) = seal_photo_with(&key, &photo).unwrap();
        assert!(blob.len() <= MAX_PHOTO_BLOB);
        assert_eq!(open_photo_with(&key, &h, &blob).unwrap(), photo);
        // Hash de outra foto, foto mexida ou outra chave: recusado.
        assert!(open_photo_with(&key, &[0u8; 32], &blob).is_err());
        let mut m = blob.clone();
        m[30] ^= 1;
        assert!(open_photo_with(&key, &photo_hash(&m), &m).is_err());
        assert!(open_photo_with(&[7u8; 32], &h, &blob).is_err());
        // A mesma foto cifrada duas vezes da hashes diferentes (nonce aleatorio).
        assert_ne!(seal_photo_with(&key, &photo).unwrap().0, h);
    }

    #[test]
    fn estado_do_operador_so_abre_com_a_semente() {
        let blob = seal_state(&[1u8; 32], b"{}");
        assert_eq!(open_state(&[1u8; 32], &blob).unwrap(), b"{}");
        assert!(open_state(&[2u8; 32], &blob).is_err());
    }

    #[test]
    fn ordem_por_medalha_e_de_entrada() {
        // O hash marca cada membro; a ordem dentro da medalha e a do indice (de entrada).
        let m = |i: u8, badge| Member { badge, photo_hash: Some([i; 32]) };
        let mut v = vec![m(1, 4), m(2, 9), m(3, 4), m(4, 1), m(5, 8)];
        sort_members(&mut v);
        let order: Vec<u8> = v.iter().map(|x| x.photo_hash.unwrap()[0]).collect();
        assert_eq!(order, [4, 1, 3, 5, 2]);
    }
}
