//! Prova de trabalho para criar uma conta no relay (contra contas em massa).
//!
//! Criar uma identidade nao custa nada (e so gerar chaves), por isso um spammer
//! podia criar milhares de contas e contornar qualquer limite por conta. Antes de
//! o relay aceitar o PRIMEIRO bundle de uma conta nova, o telemovel tem de
//! encontrar um `nonce` tal que
//!
//! ```text
//! Argon2id(m = 8 MiB, t = 1, p = 1, "QT-POW-v1" || signing_public || nonce)
//! ```
//!
//! comece por `POW_BITS` bits a zero. Em media sao `2^POW_BITS` tentativas
//! (uns segundos num telemovel, uma vez na vida da conta); o relay verifica com
//! uma so. O Argon2id usa memoria de proposito: uma placa grafica ou um ASIC
//! ganham muito menos do que com um hash simples (SHA-256).
//!
//! A prova fica ligada a chave Ed25519 da conta (`signing_public`): nao serve
//! para outra conta e pode ser reutilizada pela mesma (o telemovel guarda o
//! `nonce` e manda-o sempre que publica o bundle).

use argon2::{Algorithm, Argon2, Params, Version};
use rand_core::{OsRng, RngCore};

pub const POW_CONTEXT: &[u8] = b"QT-POW-v1";
const POW_SALT: &[u8; 16] = b"QT-POW-v1-salt-1";
/// Dificuldade: numero de bits a zero no inicio do hash.
pub const POW_BITS: u32 = 7;
/// Memoria do Argon2id por tentativa, em KiB.
pub const POW_MEMORY_KIB: u32 = 8 * 1024;

fn hasher() -> Option<Argon2<'static>> {
    let params = Params::new(POW_MEMORY_KIB, 1, 1, Some(32)).ok()?;
    Some(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// Hash da prova para esta conta e este `nonce`.
pub fn pow_hash(signing_public: &[u8; 32], nonce: u64) -> Option<[u8; 32]> {
    let mut input = Vec::with_capacity(POW_CONTEXT.len() + 32 + 8);
    input.extend_from_slice(POW_CONTEXT);
    input.extend_from_slice(signing_public);
    input.extend_from_slice(&nonce.to_be_bytes());
    let mut out = [0u8; 32];
    hasher()?.hash_password_into(&input, POW_SALT, &mut out).ok()?;
    Some(out)
}

fn leading_zero_bits(h: &[u8]) -> u32 {
    let mut bits = 0;
    for b in h {
        if *b == 0 {
            bits += 8;
        } else {
            bits += b.leading_zeros();
            break;
        }
    }
    bits
}

/// O `nonce` prova `bits` bits de trabalho para esta conta?
pub fn pow_verify(signing_public: &[u8; 32], nonce: u64, bits: u32) -> bool {
    pow_hash(signing_public, nonce).is_some_and(|h| leading_zero_bits(&h) >= bits)
}

/// Procura um `nonce` valido (a partir de um ponto aleatorio). `None` so se o
/// Argon2 falhar (nao acontece com os parametros fixos acima) ou se, por azar
/// extremo, `2^(bits + 12)` tentativas nao chegarem.
pub fn pow_solve(signing_public: &[u8; 32], bits: u32) -> Option<u64> {
    let mut nonce = OsRng.next_u64();
    let limit = 1u64 << (bits.min(40) + 12);
    for _ in 0..limit {
        if leading_zero_bits(&pow_hash(signing_public, nonce)?) >= bits {
            return Some(nonce);
        }
        nonce = nonce.wrapping_add(1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conta_bits_a_zero() {
        assert_eq!(leading_zero_bits(&[0x00, 0x00, 0x80]), 16);
        assert_eq!(leading_zero_bits(&[0x0f, 0xff]), 4);
        assert_eq!(leading_zero_bits(&[0xff]), 0);
        assert_eq!(leading_zero_bits(&[0, 0]), 16);
    }

    #[test]
    fn prova_valida_so_para_a_propria_conta() {
        // Dificuldade baixa para o teste ser rapido (o Argon2 de 8 MiB e lento sem --release).
        let me = [7u8; 32];
        let nonce = pow_solve(&me, 2).unwrap();
        assert!(pow_verify(&me, nonce, 2));
        // Outra conta: o mesmo nonce so passa por acaso (1 em 4); com 20 contas, alguma falha.
        let falhas = (0..20u8).filter(|i| !pow_verify(&[*i ^ 0x55; 32], nonce, 2)).count();
        assert!(falhas > 0);
        // Pedir mais bits do que os do hash: impossivel.
        assert!(!pow_verify(&me, nonce, 257));
    }
}
