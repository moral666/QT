//! Obtencao da passphrase da base de dados SEM a expor em `argv`.
//!
//! Antes a passphrase vinha de `--passphrase "..."`, o que a deixava visivel
//! a qualquer utilizador da maquina (`ps aux`, `/proc/<pid>/cmdline`), no
//! historico do shell e em logs de auditoria de processos. Agora:
//!
//! * por omissao e pedida de forma interativa no terminal, mascarada com `*`;
//! * para scripts/CI existe `--passphrase-stdin`: le UMA linha do stdin
//!   (ex.: `pass show qt/bob | messenger --passphrase-stdin receive ...`).
//!
//! Em nenhum caso a passphrase passa por argumentos nem por variaveis de
//! ambiente. O valor devolvido e `Zeroizing<String>`: a memoria e apagada
//! quando sai de ambito. Limitacao honesta: copias transitorias dentro do
//! `inquire`/terminal nao sao controladas por nos.

use inquire::validator::Validation;
use inquire::{InquireError, Password, PasswordDisplayMode};
use std::fmt;
use std::io::BufRead;
use zeroize::Zeroizing;

/// Comprimento minimo exigido ao CRIAR uma base de dados nova.
pub const MIN_NEW_PASSPHRASE_CHARS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Criar uma base de dados/identidade nova: pede confirmacao e aplica o minimo.
    Create,
    /// Abrir uma base de dados existente: uma unica entrada.
    Unlock,
}

#[derive(Debug)]
pub enum PassphraseError {
    /// Nao ha terminal interativo. Usar `--passphrase-stdin`.
    NotInteractive,
    /// O utilizador cancelou (Esc / Ctrl+C).
    Cancelled,
    Empty,
    TooShort,
    Io(std::io::Error),
    Prompt(String),
}

impl fmt::Display for PassphraseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInteractive => write!(f, "sem terminal interativo: usa --passphrase-stdin para scripts"),
            Self::Cancelled => write!(f, "operacao cancelada"),
            Self::Empty => write!(f, "a passphrase nao pode ser vazia"),
            Self::TooShort => write!(f, "a passphrase tem de ter pelo menos {MIN_NEW_PASSPHRASE_CHARS} caracteres"),
            Self::Io(_) => write!(f, "falha ao ler a passphrase do stdin"),
            Self::Prompt(m) => write!(f, "falha no prompt da passphrase: {m}"),
        }
    }
}

impl std::error::Error for PassphraseError {}

/// Obtem a passphrase de acordo com `mode`, interativamente ou do stdin.
pub fn obtain(mode: Mode, from_stdin: bool) -> Result<Zeroizing<String>, PassphraseError> {
    let pass = if from_stdin { read_one_line(std::io::stdin().lock())? } else { prompt(mode)? };
    validate(&pass, mode)?;
    Ok(pass)
}

pub fn validate(pass: &str, mode: Mode) -> Result<(), PassphraseError> {
    if pass.is_empty() {
        return Err(PassphraseError::Empty);
    }
    if mode == Mode::Create && pass.chars().count() < MIN_NEW_PASSPHRASE_CHARS {
        return Err(PassphraseError::TooShort);
    }
    Ok(())
}

fn prompt(mode: Mode) -> Result<Zeroizing<String>, PassphraseError> {
    let base = Password::new("Passphrase da base de dados:")
        .with_display_mode(PasswordDisplayMode::Masked) // mostra '*' por cada caracter
        .with_formatter(&|_| String::from("********")); // nunca ecoa o comprimento real

    let p = match mode {
        Mode::Unlock => base.without_confirmation(),
        Mode::Create => base
            .with_help_message("Nova base de dados: minimo de 12 caracteres. Nao ha recuperacao se a perderes.")
            .with_validator(|s: &str| {
                Ok(if s.chars().count() >= MIN_NEW_PASSPHRASE_CHARS {
                    Validation::Valid
                } else {
                    Validation::Invalid(format!("minimo de {MIN_NEW_PASSPHRASE_CHARS} caracteres").into())
                })
            })
            .with_custom_confirmation_message("Repete a passphrase:")
            .with_custom_confirmation_error_message("As passphrases nao coincidem."),
    };

    match p.prompt() {
        Ok(s) => Ok(Zeroizing::new(s)),
        Err(InquireError::NotTTY) => Err(PassphraseError::NotInteractive),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => Err(PassphraseError::Cancelled),
        Err(InquireError::IO(e)) => Err(PassphraseError::Io(e)),
        Err(e) => Err(PassphraseError::Prompt(e.to_string())),
    }
}

/// Le exatamente uma linha (sem o `\n` / `\r\n` final). O buffer intermedio
/// e `Zeroizing`, para nao deixar copias da passphrase na heap.
pub fn read_one_line<R: BufRead>(mut r: R) -> Result<Zeroizing<String>, PassphraseError> {
    let mut line = Zeroizing::new(String::new());
    r.read_line(&mut line).map_err(PassphraseError::Io)?;
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_uma_linha_e_remove_fim_de_linha() {
        assert_eq!(read_one_line(&b"segredo forte!\r\nlixo"[..]).unwrap().as_str(), "segredo forte!");
        assert_eq!(read_one_line(&b"sem newline"[..]).unwrap().as_str(), "sem newline");
        // Espacos fazem parte da passphrase: nao sao cortados.
        assert_eq!(read_one_line(&b" a b \n"[..]).unwrap().as_str(), " a b ");
    }

    #[test]
    fn validacao() {
        assert!(matches!(validate("", Mode::Unlock), Err(PassphraseError::Empty)));
        assert!(validate("curta", Mode::Unlock).is_ok(), "bases antigas podem ter passphrases curtas");
        assert!(matches!(validate("curta", Mode::Create), Err(PassphraseError::TooShort)));
        assert!(validate("uma-passphrase-longa", Mode::Create).is_ok());
    }
}
