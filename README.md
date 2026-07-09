# secure-messenger (nome provisório)

Mensageiro E2EE open source, focado em privacidade extrema, resistência a
backdoors e anonimato no registo. Ver `docs/protocol-spec.md` para a
especificação completa do protocolo e `apresentacao-projeto.md` (na raiz do
pacote entregue) para uma explicação em linguagem simples.

## Estado atual do projeto

✅ **Núcleo criptográfico compilado e testado de verdade** (não é só pseudocódigo):
X3DH clássico + assinatura Ed25519 da signed pre-key + one-time pre-keys +
Double Ratchet — 4 testes de integração a passar. PQXDH (X25519 + ML-KEM/Kyber,
pós-quântico) também implementado e testado, atrás da feature `pq` (ver
secção "Compilar" abaixo).

✅ **Servidor/relay também compilado e testado de verdade**: fila de
mensagens + diretório de pre-keys, comunicando via o canal Noise/WebSocket
já testado. O teste `server/tests/relay_flow.rs` prova a capacidade mais
importante de um relay: **entrega assíncrona** — Alice liga-se, envia uma
mensagem para Bob, e desliga-se; só depois Bob se liga (numa ligação TCP
completamente separada) e recebe a mensagem da fila, decifrando-a
corretamente com o Double Ratchet. O servidor nunca vê o conteúdo em texto
plano em nenhum momento.

✅ **Demo de terminal, visível e executável de verdade**: `cli/src/bin/messenger_demo.rs`
corre a stack inteira (core + transport + server) e imprime, passo a passo,
uma conversa E2EE completa entre "Alice" e "Bob" — é o primeiro momento em
que o projeto se sente um pouco como um produto, não só testes automáticos.

✅ **Armazenamento local encriptado também compilado e testado de verdade**:
SQLCipher (ligado à biblioteca do sistema, `libsqlcipher`), guardando a
identidade e as sessões de Double Ratchet. Testado em
`storage/tests/persistence.rs`: a identidade e as sessões sobrevivem a
fechar e reabrir a base de dados, uma passphrase errada não consegue ler
nada, e o ficheiro em disco não tem nenhuma assinatura ou conteúdo legível
(confirmado manualmente — não é um SQLite comum, é mesmo cifrado).

✅ **CLI persistente, testado com processos separados de verdade**:
`cli/src/bin/messenger.rs` — `identity`, `register`, `send`, `receive` como
comandos completamente independentes, cada execução um processo novo, com
o estado (identidade + sessão) guardado em SQLCipher entre eles. Testado
manualmente correndo os quatro comandos como processos reais separados:
Alice enviou, Bob recebeu e decifrou corretamente — depois uma segunda
mensagem confirmou que a sessão continua no sítio certo entre execuções.

🚧 **Ainda falta**, antes de qualquer uso real:

- [ ] Sealed sender (o servidor ainda associa `to`/`user_id` diretamente)
- [ ] Bindings FFI (uniffi) para Android/iOS - a peça que finalmente traz
      isto para um telemóvel
- [ ] Persistência real no servidor (a fila atual é em memória — perde-se
      se o processo reiniciar)
- [ ] Passphrase da base de dados vinda do Android Keystore / iOS Secure
      Enclave, em vez de argumento de linha de comandos (ver cli/, storage/)
- [ ] TLS (`wss://`) e persistência da chave estática Noise do servidor
      entre reinícios
- [ ] One-time pre-key só usada uma vez de verdade (o CLI atual reutiliza
      sempre a mesma - simplificação documentada, não é o comportamento
      correto de produção)
- [ ] Auditoria de segurança externa

**Não usar em produção ainda** — falta a app cliente e o endurecimento do
servidor para produção real.

## Estrutura do repositório

Workspace Cargo com 3 crates:

```
Cargo.toml      # workspace root

core/           # Núcleo E2EE: X3DH + Double Ratchet (compilado e testado)
  src/
    primitives.rs   # DH, HKDF, AEAD, assinaturas Ed25519
    x3dh.rs         # Handshake clássico (com assinatura + one-time pre-key)
    pqxdh.rs         # Variante pós-quântica (atrás da feature "pq")
    ratchet.rs       # Double Ratchet (forward secrecy contínua)
  tests/
    full_flow.rs     # 4 testes: fluxo completo, assinatura/mensagem adulterada, sem OTK
    pqxdh_flow.rs     # Teste do fluxo PQXDH (exige --features pq)

transport/      # Camada de transporte: Noise sobre WebSocket (compilado e testado)
  src/
    noise_session.rs # Wrapper sobre o Noise Protocol Framework (crate snow)
    ws_transport.rs   # Liga o handshake/transporte Noise a WebSocket real

demo/           # Crate de demonstração - liga core + transport num teste completo
  tests/
    e2e_over_real_websocket.rs  # Mensagem E2EE real, através de WebSocket real

server/         # Servidor/relay: fila de mensagens + diretório de pre-keys
  src/
    protocol.rs      # Mensagens cliente<->servidor (ClientMessage/ServerMessage)
    store.rs         # Armazenamento em memória (a substituir por Redis/DB antes de produção)
    connection.rs     # Handshake Noise + loop de processamento por ligação
    bin/relay_server.rs  # Binário standalone (cargo run --bin relay_server)
  tests/
    relay_flow.rs     # Prova a entrega assíncrona: liga, envia, desliga, o outro liga depois e recebe

cli/            # Demo de terminal - vê a conversa a acontecer, em texto
  src/
    wire_format.rs    # Serialização do PreKeyBundle para viajar pela rede
    bin/messenger_demo.rs  # `cargo run --bin messenger_demo` - a experiência completa

storage/        # Armazenamento local encriptado (SQLCipher)
  src/lib.rs        # save/load de identidade e sessões, base de dados cifrada
  tests/
    persistence.rs   # Sobrevive a fechar/reabrir; passphrase errada falha

docs/
  protocol-spec.md         # Especificação do protocolo
  poc_ratchet_python.py    # PoC inicial em Python (referência histórica)

android/        # (a criar) App Android nativo consumindo core/ e transport/ via FFI
ios/            # (a criar) App iOS nativo
desktop/        # (a criar) App Tauri
server/         # (repositório separado - ver justificativa em CONTRIBUTING.md)
```

## Como compilar e testar

Requisitos: Rust estável (via [rustup](https://rustup.rs), recomendado) —
qualquer versão recente serve para a maior parte do workspace; a feature
`pq` do `core` (pós-quântico) exige especificamente **Rust 1.81 ou mais
recente**.

```bash
# Testar tudo (core + transport + demo), sem a feature pq
cargo test --workspace

# Incluindo PQXDH (ML-KEM/Kyber) no core - exige Rust 1.81+
cargo test -p secure_messenger_core --features pq

# Só o teste de ponta a ponta (E2EE real sobre WebSocket real em localhost)
cargo test -p secure_messenger_demo
```

O teste mais interessante para veres a arquitetura em ação é
`demo/tests/e2e_over_real_websocket.rs`: sobe um servidor WebSocket real em
`localhost`, faz o handshake Noise, e envia uma mensagem já cifrada pelo
Double Ratchet através dele - as duas camadas de segurança a funcionar
juntas, com tráfego de rede real (não simulado em memória).

`server/tests/relay_flow.rs` vai um passo mais além: prova a entrega
assíncrona (Alice envia enquanto Bob está offline; Bob liga-se depois,
noutra ligação TCP, e recebe da fila).

Para correr o servidor/relay como processo standalone (ex.: para ligares
uma app cliente a ele manualmente):

```bash
cargo run --bin relay_server
# imprime a porta e a chave publica Noise do servidor
```

**A forma mais rápida de veres tudo isto em ação** é a demo de terminal:

```bash
cargo run -p secure_messenger_cli --bin messenger_demo
```

Isto sobe o seu próprio servidor local e mostra, passo a passo, uma
conversa completa entre "Alice" e "Bob" — geração de identidade, X3DH,
Double Ratchet, envio através do relay, entrega assíncrona, e decifragem
do outro lado — tudo impresso no terminal.

**Para a versão persistente de verdade** (comandos separados, cada um um
processo novo, estado guardado entre eles), primeiro sobe um servidor:

```bash
cargo run --bin relay_server &
```

Depois, em terminais/execuções separadas:

```bash
# Bob cria a sua identidade e publica-a
cargo run -p secure_messenger_cli --bin messenger -- identity --db bob.sqlite --passphrase "escolhe-uma-passphrase-forte"
cargo run -p secure_messenger_cli --bin messenger -- register --db bob.sqlite --passphrase "escolhe-uma-passphrase-forte" --server ws://127.0.0.1:9443

# Alice cria a sua identidade e envia uma mensagem (usa o ID que o comando 'identity' do Bob imprimiu)
cargo run -p secure_messenger_cli --bin messenger -- identity --db alice.sqlite --passphrase "outra-passphrase-forte"
cargo run -p secure_messenger_cli --bin messenger -- send --db alice.sqlite --passphrase "outra-passphrase-forte" --to <ID-DO-BOB> --message "Ola!" --server ws://127.0.0.1:9443

# Bob, mais tarde, processo completamente novo
cargo run -p secure_messenger_cli --bin messenger -- receive --db bob.sqlite --passphrase "escolhe-uma-passphrase-forte" --server ws://127.0.0.1:9443
```

O ID de cada pessoa é derivado automaticamente da sua chave pública de
identidade (não é escolhido por ela) — consistente com o objetivo de não
depender de nomes de utilizador.

## Licença

AGPL-3.0-or-later — ver `LICENSE`. Escolhida deliberadamente: qualquer
serviço que rode uma versão modificada deste código, mesmo apenas como
serviço hospedado, é obrigado a disponibilizar o código-fonte das suas
modificações. Isto fecha a brecha de "embrace, extend, fechar" que a GPL
comum não cobre para software rodado como serviço.

## Contribuir

Ver `CONTRIBUTING.md`. Mudanças em `core/` (qualquer coisa relacionada a
criptografia) exigem revisão extra e testes de vetores antes de merge.

## Reportar vulnerabilidades

Ver `SECURITY.md` — **não abra issues públicas para vulnerabilidades.**
