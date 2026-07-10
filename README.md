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

✅ **Sealed sender também implementado e testado de verdade**: o servidor
já não vê quem enviou cada mensagem, só para quem é entregue. Cada
mensagem leva um pequeno envelope cifrado (`core::sealed_sender`) que só o
destinatário, com a sua chave privada de identidade, consegue abrir.
Testado a três níveis: no `core/` isoladamente (destinatário certo abre,
qualquer outra chave falha), no `server/` (confirmando que o campo guardado
na fila não é a string em bruto do remetente), e na demo de terminal
(que agora mostra explicitamente "remetente ainda selado" até Bob abrir o
envelope).

✅ **Bindings FFI (uniffi) gerados e testados de verdade**: `ffi/` expõe
uma API funcional do núcleo (geração de chaves, X3DH, Double Ratchet,
sealed sender) via [uniffi](https://mozilla.github.io/uniffi-rs/) — a
mesma ferramenta usada pelo Signal e pela Mozilla. Gerámos bindings para
**Kotlin** (Android), **Swift** (iOS) e **Python**, e corremos os 7 testes
mais importantes do projeto (X3DH, Double Ratchet, sealed sender,
assinatura/chave errada a falhar corretamente) **a partir do Python**,
contra os bindings gerados — a prova de que o FFI funciona de verdade fora
do Rust, não só em teoria.

✅ **Persistência real no servidor (Redis)**: a fila de mensagens e o
diretório de pre-keys já não vivem em memória — estão em Redis de
verdade, com TTL automático nas filas (30 dias sem serem levantadas).
**Testado da forma mais convincente possível**: registei o bundle de Bob,
matei o processo do servidor por completo (`kill -9`), confirmei no
Redis que os dados continuavam lá, subi um servidor novo (chave Noise
diferente, processo diferente), e a Alice conseguiu enviar-lhe uma
mensagem sem ninguém se ter de registar outra vez.

✅ **Chave estática Noise do servidor também persistida**: já não é gerada
de novo a cada arranque — fica guardada em disco (`relay_noise_key.bin`,
permissões `0600`), gerada uma única vez no primeiro arranque. **Testado
matando o processo com `kill -9` e arrancando um novo**: a chave pública
impressa foi exatamente a mesma nos dois arranques, confirmando que o
"pinning" entre clientes e servidor agora sobrevive a reinícios de verdade.

🚧 **Ainda falta**, antes de qualquer uso real:

- [ ] Apps Android/iOS de verdade que consomem os bindings gerados (o FFI
      em si já está pronto - falta a camada de UI nativa à volta dele)
- [ ] Passphrase da base de dados vinda do Android Keystore / iOS Secure
      Enclave, em vez de argumento de linha de comandos (ver cli/, storage/)
- [ ] TLS (`wss://`) - o transporte ainda usa `ws://` puro, adequado só
      para desenvolvimento local
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
    sealed_sender.rs # Esconde a identidade do remetente do servidor
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

server/         # Servidor/relay: fila de mensagens (Redis) + diretório de pre-keys
  src/
    protocol.rs      # Mensagens cliente<->servidor (ClientMessage/ServerMessage)
    store.rs         # Armazenamento em Redis real, com TTL automático nas filas
    connection.rs     # Handshake Noise + loop de processamento por ligação
    bin/relay_server.rs  # Binário standalone (cargo run --bin relay_server)
  tests/
    relay_flow.rs     # Prova a entrega assíncrona contra Redis real

cli/            # Demo de terminal - vê a conversa a acontecer, em texto
  src/
    wire_format.rs    # Serialização do PreKeyBundle para viajar pela rede
    bin/messenger_demo.rs  # `cargo run --bin messenger_demo` - a experiência completa

storage/        # Armazenamento local encriptado (SQLCipher)
  src/lib.rs        # save/load de identidade e sessões, base de dados cifrada
  tests/
    persistence.rs   # Sobrevive a fechar/reabrir; passphrase errada falha

ffi/            # Bindings FFI (uniffi) - Kotlin, Swift, Python
  src/lib.rs        # API funcional (bytes dentro, bytes fora) exportada via uniffi
  generate_bindings.sh  # Compila + gera os bindings + corre o teste Python
  tests/
    test_ffi_bindings.py  # 7 testes reais, a partir do Python, via bindings gerados
  bindings/       # GERADO (não commitado) - ver .gitignore

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
recente**. Os testes do `server/` (e o binário `relay_server`) precisam de
um **Redis a correr em `localhost:6379`**:

```bash
sudo apt install redis-server libsqlcipher-dev
redis-server --daemonize yes
```

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

**Para gerar e testar os bindings FFI** (Kotlin/Swift/Python):

```bash
./ffi/generate_bindings.sh
```

Isto compila o núcleo, gera os bindings nas três linguagens em
`ffi/bindings/`, e corre um teste real em Python contra eles (X3DH,
Double Ratchet, sealed sender - os mesmos conceitos que uma app Android ou
iOS real usaria através dos ficheiros `.kt`/`.swift` gerados).

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
