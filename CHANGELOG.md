# Changelog

Formato inspirado em [Keep a Changelog](https://keepachangelog.com/).
Datas aproximadas — este projeto ainda não segue versionamento semântico
formal (não há releases numeradas ainda, ver `ROADMAP.md`).

## [Não lançado]

### Adicionado
- Modelo de ameaças completo (`docs/threat-model.md`)
- Roadmap dedicado (`ROADMAP.md`)
- Este changelog
- Testes de robustez contra bytes malformados: `core/tests/fuzz_lite.rs`
  (40.000 tentativas), `cli/tests/fuzz_lite.rs` (20.000 tentativas), e
  alvos reais de `cargo-fuzz` em `core/fuzz/` (prontos, não corridos
  ainda por falta de Rust nightly no ambiente de desenvolvimento)

### Corrigido
- `SECURITY.md` já não tem placeholders por preencher — usa o mecanismo
  de report privado do GitHub em vez de um email/PGP que nunca existiu
- **Crash real corrigido**: `cli/src/bin/messenger.rs` entrava em pânico
  ao processar uma mensagem vazia ou demasiado curta vinda do servidor
  (indexação direta `bytes[0]` sem verificação de tamanho) — encontrado
  através do trabalho de testes de robustez acima. Corrigido para
  devolver um erro controlado, sem derrubar o processo nem impedir o
  processamento das restantes mensagens da fila.

## Núcleo e infraestrutura (fundação técnica)

Por ordem de construção, não de data exata (este projeto começou como
uma conversa de arquitetura antes de existir código):

- **`core/`**: X3DH clássico, depois assinatura Ed25519 da signed
  pre-key, one-time pre-keys, PQXDH (X25519 + ML-KEM/Kyber), Double
  Ratchet, e por fim Sealed Sender — cada peça com testes automáticos
  dedicados, incluindo casos que devem falhar (assinatura adulterada,
  mensagem adulterada, chave errada)
- **`transport/`**: Noise Protocol Framework (`Noise_XX_25519_ChaChaPoly_SHA256`)
  sobre WebSocket, testado com uma ligação real em `localhost`
- **`server/`**: relay com fila de mensagens e diretório de pre-keys —
  primeiro em memória, depois migrado para Redis real, com TTL
  automático; testado sobrevivendo a `kill -9` do processo
- **`storage/`**: armazenamento local em SQLCipher — identidade e
  sessões, testado com passphrase errada a falhar e o ficheiro em disco
  confirmado como genuinamente cifrado (sem a assinatura padrão do SQLite)
- **`cli/`**: primeiro uma demo num único processo, depois um cliente
  persistente a sério (`identity`/`register`/`send`/`receive`), testado
  com processos completamente separados a comunicar entre si
- **`ffi/`**: bindings uniffi para Kotlin, Swift e Python — testados via
  Python primeiro, depois com um `NoiseSession` com estado para expor
  também a camada de transporte, e finalmente testado num Android real
  (emulador)
- **Persistência da chave estática Noise do servidor** entre reinícios
  (antes gerada de novo a cada arranque)

## Organização e confiança

- Projeto renomeado de "secure-messenger" para **QT**
- Licença AGPL-3.0 completa (antes tinha um placeholder por preencher)
- README reorganizado com moldura honesta de "protótipo técnico", não
  "produto acabado" — logo na primeira frase
- Versão em inglês do README (`README.en.md`)
- `.gitignore` criado (faltava desde o início)
- CI (GitHub Actions) configurado: testes do workspace inteiro, feature
  pós-quântica, bindings FFI via Python, `cargo clippy` estrito,
  `cargo audit`
