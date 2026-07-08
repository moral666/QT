# Contribuir para o projeto

## Estrutura mono-repo vs multi-repo

Este repositório (`secure-messenger`) contém o núcleo criptográfico e, no
futuro, os clientes (Android/iOS/Desktop), porque todos consomem a mesma
versão de `core/` via FFI — mantê-los juntos evita divergência entre
plataformas. O servidor/relay vive em repositório separado
(`secure-messenger-server`), já que tem ciclo de deploy e contribuidores
potencialmente diferentes (quem quer só fazer self-host não precisa clonar
os clientes mobile).

## Git flow

- `main` — protegido, apenas releases. Requer aprovação de mantenedor.
- `develop` — branch de integração. PRs normais entram aqui.
- `feature/nome-da-feature` — branches de trabalho, a partir de `develop`.

Regras de PR:
- 2 aprovações mínimas para merge em `develop`.
- **PRs que tocam `core/src/primitives.rs`, `core/src/x3dh.rs` ou
  `core/src/ratchet.rs` exigem aprovação de um mantenedor designado como
  responsável por criptografia**, além das 2 aprovações normais.
- Commits assinados com GPG são obrigatórios (`git commit -S`).
- Use Conventional Commits: `feat:`, `fix:`, `security:`, `docs:`, `refactor:`.

## Regras específicas para código de criptografia

1. Nenhuma primitiva criptográfica nova sem justificar por que uma
   biblioteca existente auditada (ex.: as já usadas em `Cargo.toml`) não
   serve.
2. Toda mudança em `ratchet.rs` ou `x3dh.rs` deve vir acompanhada de teste
   em `core/tests/` cobrindo o caso (incluir também um teste que prove que
   o caso *inválido* falha, como em `mensagem_adulterada_deve_falhar`).
3. Nunca faça `println!`/log de chaves privadas, chaves de mensagem, ou
   material derivado, nem em modo debug.
4. Use `zeroize` para qualquer buffer que contenha material de chave que
   saia de escopo.

## Rodando os testes localmente

```bash
cd core
cargo test              # testes funcionais
cargo clippy -- -D warnings   # lint estrito
cargo audit              # checagem de vulnerabilidades conhecidas em dependências
```

## Antes de abrir um PR

- [ ] `cargo test` passa
- [ ] `cargo clippy` sem warnings
- [ ] Se tocou em `core/`, adicionou/atualizou testes
- [ ] Descrição do PR explica *por que*, não só *o quê*
