# QT

Mensageiro E2EE open source, focado em privacidade extrema, resistência a
backdoors (tipo Chat Control da UE) e anonimato no registo (sem
telefone/email). Ver:

- [`apresentacao-projeto.md`](../apresentacao-projeto.md) — explicação em linguagem simples, sem jargão
- [`docs/protocol-spec.md`](docs/protocol-spec.md) — especificação técnica completa do protocolo

## Estado do projeto

Toda a stack de backend está construída, compilada e testada de verdade
(não é pseudocódigo). Falta a camada de produto final (apps nativas
publicáveis, hardening para produção).

| Componente | O que faz | Estado |
|---|---|---|
| `core/` | X3DH + PQXDH (pós-quântico) + Double Ratchet + Sealed Sender | ✅ Testado (14+ testes automáticos) |
| `transport/` | Noise Protocol sobre WebSocket | ✅ Testado com WebSocket real |
| `server/` | Relay: fila de mensagens + diretório de pre-keys, em Redis | ✅ Testado, incluindo sobrevivência a `kill -9` |
| `storage/` | Armazenamento local encriptado (SQLCipher) | ✅ Testado, incluindo passphrase errada a falhar |
| `cli/` | Cliente de terminal persistente (`identity`/`register`/`send`/`receive`) | ✅ Testado com processos separados reais |
| `ffi/` | Bindings uniffi (Kotlin/Swift/Python) do núcleo + transporte | ✅ Testado via Python |
| App Android | Consome os bindings FFI, ecrã de demonstração | ✅ Corre num emulador real (ainda só local, sem repositório próprio) |

**Ainda por fazer**, antes de qualquer uso real:
- [ ] Ecrã de conversa completo na app Android (contactos, chat — hoje é só um botão de demonstração)
- [ ] TLS (`wss://`) — o transporte usa `ws://` puro, adequado só para rede local/testes
- [ ] Servidor alojado publicamente (hoje só corre em localhost/rede local)
- [ ] Federação (vários servidores independentes, em vez de um único)
- [ ] Passphrase da base de dados vinda do Keystore/Secure Enclave, não de argumento de linha de comandos
- [ ] One-time pre-key consumida uma única vez de verdade (o CLI atual reutiliza a mesma)
- [ ] Auditoria de segurança externa

## Estrutura do repositório

Workspace Cargo com 7 crates:

```
core/        Núcleo E2EE — X3DH, PQXDH, Double Ratchet, Sealed Sender
transport/   Noise Protocol sobre WebSocket
server/      Relay: fila de mensagens (Redis) + diretório de pre-keys
storage/     Armazenamento local encriptado (SQLCipher)
cli/         Cliente de terminal (demo + persistente)
demo/        Prova de conceito ligando core+transport num teste único
ffi/         Bindings uniffi (Kotlin, Swift, Python)
docs/        Especificação do protocolo e histórico de design
```

Cada crate tem os seus próprios testes em `tests/` — ver a tabela acima
para o que cada um prova. A app Android existe já (testada num emulador),
mas ainda vive só localmente na máquina de desenvolvimento — quando for
publicada, deve ficar num repositório à parte que consome `ffi/` como
dependência externa, pela mesma lógica de separação descrita em
`CONTRIBUTING.md`.

## Como compilar e testar

**Requisitos:**
```bash
sudo apt install redis-server libsqlcipher-dev
redis-server --daemonize yes
```
Rust via [rustup](https://rustup.rs) — qualquer versão recente serve,
exceto a feature `pq` do `core` (pós-quântico), que exige **Rust 1.81+**.

**Correr os testes:**
```bash
cargo test --workspace                              # tudo, exceto pq
cargo test -p qt_core --features pq   # incluindo pós-quântico
```

**Ver a arquitetura em ação** (a forma mais rápida de perceber o projeto):
```bash
cargo run -p qt_cli --bin messenger_demo
```
Mostra, passo a passo, uma conversa E2EE completa entre "Alice" e "Bob" —
identidade, X3DH, Double Ratchet, sealed sender, envio pelo relay, entrega
assíncrona, decifragem — tudo impresso no terminal.

**Usar como cliente persistente de verdade** (processos separados,
estado guardado entre eles):
```bash
cargo run --bin relay_server &

cargo run -p qt_cli --bin messenger -- identity --db bob.sqlite --passphrase "..."
cargo run -p qt_cli --bin messenger -- register --db bob.sqlite --passphrase "..." --server ws://127.0.0.1:9443

cargo run -p qt_cli --bin messenger -- identity --db alice.sqlite --passphrase "..."
cargo run -p qt_cli --bin messenger -- send --db alice.sqlite --passphrase "..." --to <ID-DO-BOB> --message "Ola!" --server ws://127.0.0.1:9443

cargo run -p qt_cli --bin messenger -- receive --db bob.sqlite --passphrase "..." --server ws://127.0.0.1:9443
```
O ID de cada pessoa é derivado da sua chave pública de identidade — não é
escolhido, para não depender de "nomes de utilizador".

**Gerar os bindings FFI** (Kotlin/Swift/Python):
```bash
./ffi/generate_bindings.sh
```

## Licença

AGPL-3.0-or-later — ver `LICENSE`. Qualquer serviço que rode uma versão
modificada deste código, mesmo só como serviço hospedado, é obrigado a
publicar o código-fonte das suas modificações.

## Contribuir e reportar problemas

- `CONTRIBUTING.md` — processo de PR, git flow, regras extra para código de criptografia
- `SECURITY.md` — divulgação responsável de vulnerabilidades (**não abrir issues públicas**)
- `docs/protocol-spec.md` — especificação técnica detalhada, secção a secção
