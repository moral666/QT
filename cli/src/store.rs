//! Ficheiros locais do `messenger` (CLI), ao lado da base de dados:
//!
//! * `<db>.salt`     - 16 bytes aleatorios (nao secretos) para o Argon2id;
//! * `<db>.contacts` - lista de contactos (nome -> QT Public ID), CIFRADA.
//!
//! A passphrase passa por Argon2id (64 MiB, 3 passagens) e da 64 bytes:
//! os primeiros 32 sao a chave do SQLCipher (a mesma API `QtClient::open`
//! que as apps usam, onde a chave vem do Keystore/Secure Enclave) e os
//! outros 32 cifram a lista de contactos (ChaCha20-Poly1305). Nada disto
//! fica em claro no disco; os ficheiros sao criados com permissao 0600.

use argon2::{Algorithm, Argon2, Params, Version};
use qt_core::primitives::{aead_decrypt, aead_encrypt};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

const SALT_LEN: usize = 16;
const CONTACTS_AAD: &[u8] = b"QT-CLI-CONTACTS-v1";
/// Parametros Argon2id (RFC 9106, segunda recomendacao): 64 MiB, t=3, p=1.
const ARGON_MEM_KIB: u32 = 64 * 1024;
const ARGON_PASSES: u32 = 3;

pub struct Keys {
    pub db: Zeroizing<Vec<u8>>,
    contacts: Zeroizing<[u8; 32]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub name: String,
    pub public_id: String,
}

impl Contact {
    /// Identificador usado pelo QtClient (chave publica de identidade em hex).
    pub fn contact_id(&self) -> String {
        self.public_id.get(4..68).unwrap_or_default().to_string()
    }
}

pub fn salt_path(db: &str) -> PathBuf {
    PathBuf::from(format!("{db}.salt"))
}

pub fn contacts_path(db: &str) -> PathBuf {
    PathBuf::from(format!("{db}.contacts"))
}

/// Deriva as chaves a partir da passphrase e do sal.
pub fn derive_keys(passphrase: &str, salt: &[u8]) -> Result<Keys, String> {
    let params = Params::new(ARGON_MEM_KIB, ARGON_PASSES, 1, Some(64)).map_err(|e| e.to_string())?;
    let mut out = Zeroizing::new([0u8; 64]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), salt, &mut out[..])
        .map_err(|e| e.to_string())?;
    let mut contacts = Zeroizing::new([0u8; 32]);
    contacts.copy_from_slice(&out[32..]);
    Ok(Keys { db: Zeroizing::new(out[..32].to_vec()), contacts })
}

/// Cria o sal de uma base de dados NOVA. Recusa se ja existir base ou sal
/// (nunca se gera um sal novo por cima: a base de dados deixaria de abrir).
pub fn create_salt(db: &str) -> Result<Vec<u8>, String> {
    if Path::new(db).exists() || salt_path(db).exists() {
        return Err(format!("ja existe uma base de dados em {db}"));
    }
    let mut salt = vec![0u8; SALT_LEN];
    getrandom::getrandom(&mut salt).map_err(|_| "gerador aleatorio indisponivel".to_string())?;
    write_private(&salt_path(db), &salt)?;
    Ok(salt)
}

pub fn load_salt(db: &str) -> Result<Vec<u8>, String> {
    if !Path::new(db).exists() {
        return Err(format!("nao existe base de dados em {db}: cria uma com `messenger identity --db {db}`"));
    }
    let salt = std::fs::read(salt_path(db)).map_err(|_| {
        format!(
            "falta o ficheiro {}: esta base de dados e do formato antigo do messenger \
             (incompativel com o relay atual). Cria uma nova com outro --db.",
            salt_path(db).display()
        )
    })?;
    if salt.len() != SALT_LEN {
        return Err("ficheiro de sal com tamanho invalido".into());
    }
    Ok(salt)
}

pub fn load_contacts(db: &str, keys: &Keys) -> Result<Vec<Contact>, String> {
    let path = contacts_path(db);
    let blob = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("nao foi possivel ler a lista de contactos".into()),
    };
    let plain = Zeroizing::new(
        aead_decrypt(&keys.contacts, &blob, CONTACTS_AAD).map_err(|_| "lista de contactos danificada".to_string())?,
    );
    let text = std::str::from_utf8(&plain).map_err(|_| "lista de contactos danificada".to_string())?;
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let (name, id) = line.split_once('\t').ok_or("lista de contactos danificada")?;
        out.push(Contact { name: name.to_string(), public_id: id.to_string() });
    }
    Ok(out)
}

pub fn save_contacts(db: &str, keys: &Keys, contacts: &[Contact]) -> Result<(), String> {
    let mut text = Zeroizing::new(String::new());
    for c in contacts {
        text.push_str(&c.name);
        text.push('\t');
        text.push_str(&c.public_id);
        text.push('\n');
    }
    let blob = aead_encrypt(&keys.contacts, text.as_bytes(), CONTACTS_AAD).map_err(|_| "falha ao cifrar".to_string())?;
    write_private(&contacts_path(db), &blob)
}

/// Nome de contacto valido: 1..=40 caracteres, sem controlos (tab, newline...).
pub fn valid_name(name: &str) -> bool {
    let n = name.chars().count();
    (1..=40).contains(&n) && !name.chars().any(|c| c.is_control()) && !name.starts_with("qt1:")
}

/// Escrita atomica (ficheiro temporario + rename) com permissao 0600.
fn write_private(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(|e| format!("nao foi possivel escrever {}: {e}", tmp.display()))?;
        f.write_all(data).and_then(|_| f.sync_all()).map_err(|e| format!("falha ao gravar: {e}"))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("falha ao gravar {}: {e}", path.display()))
}

/// Restringe a base de dados a este utilizador (0600).
pub fn restrict_permissions(path: &str) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdb(n: &str) -> String {
        let p = std::env::temp_dir().join(format!("qt_cli_store_{n}_{}.sqlite", std::process::id()));
        let s = p.to_string_lossy().to_string();
        for f in [s.clone(), format!("{s}.salt"), format!("{s}.contacts")] {
            let _ = std::fs::remove_file(f);
        }
        s
    }

    #[test]
    fn chaves_deterministicas_e_separadas() {
        let a = derive_keys("uma passphrase longa", &[1; 16]).unwrap();
        let b = derive_keys("uma passphrase longa", &[1; 16]).unwrap();
        let c = derive_keys("outra passphrase!!!", &[1; 16]).unwrap();
        assert_eq!(*a.db, *b.db);
        assert_ne!(*a.db, *c.db);
        assert_ne!(&a.db[..], &a.contacts[..], "chave da base e dos contactos sao diferentes");
    }

    #[test]
    fn contactos_cifrados_e_ida_e_volta() {
        let db = tmpdb("c");
        std::fs::write(&db, b"x").unwrap();
        let k = derive_keys("passphrase de teste", &[2; 16]).unwrap();
        let list = vec![Contact { name: "Ana Silva".into(), public_id: format!("qt1:{}", "ab".repeat(64)) }];
        save_contacts(&db, &k, &list).unwrap();
        let raw = std::fs::read(contacts_path(&db)).unwrap();
        assert!(!raw.windows(3).any(|w| w == b"Ana"), "o nome nao fica em claro");
        assert_eq!(load_contacts(&db, &k).unwrap(), list);
        let wrong = derive_keys("passphrase errada!", &[2; 16]).unwrap();
        assert!(load_contacts(&db, &wrong).is_err());
    }

    #[test]
    fn sal_nunca_e_recriado_por_cima() {
        let db = tmpdb("s");
        let s = create_salt(&db).unwrap();
        std::fs::write(&db, b"x").unwrap();
        assert!(create_salt(&db).is_err());
        assert_eq!(load_salt(&db).unwrap(), s);
    }

    #[test]
    fn nomes() {
        assert!(valid_name("Ana"));
        assert!(!valid_name(""));
        assert!(!valid_name("a\tb"));
        assert!(!valid_name("qt1:abc"));
        assert!(!valid_name(&"x".repeat(41)));
    }
}
