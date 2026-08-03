# Especificação do Protocolo (rascunho v0.1)

## Estado deste documento

Rascunho inicial. Cobre o que já está implementado, **compilado e testado**
em `core/`, e o que falta antes de qualquer uso real. Não é ainda uma
especificação formal revisada por pares.

## Índice

1. [Handshake inicial](#1-handshake-inicial--implementado-e-testado) — X3DH e PQXDH
2. [Double Ratchet](#2-double-ratchet--implementado) — cifra contínua, forward secrecy
3. [Camada de transporte](#3-camada-de-transporte--implementada-e-testada) — Noise sobre WebSocket
4. [Servidor/relay](#4-servidorrelay--implementado-e-testado) — fila de mensagens, Redis
5. [Armazenamento local](#5-armazenamento-local-no-cliente--implementado-e-testado) — SQLCipher
6. [Bindings FFI](#6-bindings-ffi-androidiospython--implementado-e-testado) — Kotlin/Swift/Python
7. [Sealed sender](#7-sealed-sender--implementado-e-testado) — esconder o remetente do servidor

*(Se preferires uma explicação sem jargão técnico, começa antes por
[`apresentacao-projeto.md`](../../apresentacao-projeto.md) — este
documento aqui é a referência técnica detalhada, não a porta de entrada.)*

## 1. Handshake inicial — implementado e testado

Duas variantes, ambas em `core/`, ambas com testes de integração a passar:

**X3DH clássico** (`core/src/x3dh.rs`) — 3-4x Diffie-Hellman (X25519) via
HKDF-SHA256, com:
- Assinatura Ed25519 da signed pre-key, verificada pelo iniciador antes de
  confiar nela (protege contra um servidor malicioso substituir a chave).
- One-time pre-key opcional (DH4), com degradação graciosa quando o
  servidor não tem nenhuma disponível.
- Testado em `core/tests/full_flow.rs` (4 testes, incluindo casos que
  devem falhar: assinatura adulterada, mensagem adulterada).

**PQXDH** (`core/src/pqxdh.rs`, atrás da feature `pq`) — a mesma estrutura,
com um encapsulamento ML-KEM-768 (Kyber, FIPS 203) combinado no HKDF final,
para resistência a "colher agora, decifrar depois" por um adversário com
computador quântico futuro. Exige Rust 1.81+ (dependência `hybrid-array`).
Testado em `core/tests/pqxdh_flow.rs`.

**Pendente**: nenhum dos dois expõe ainda uma API para o servidor consumir
one-time pre-keys de um pool (isso é lógica de servidor, fora do escopo de
`core/` — ver secção 4 do roadmap geral do projeto).

## 2. Double Ratchet — implementado

Implementado em `core/src/ratchet.rs`, seguindo a especificação de Perrin/
Marlinspike (Signal, 2016): symmetric-key ratchet (uma chave nova por
mensagem, via `kdf_chain_key`) + DH ratchet (nova geração de chaves a cada
"turno" da conversa, via `kdf_root_key`).

Garantias já cobertas por teste (`core/tests/full_flow.rs`):
- Mensagens em sequência decifram corretamente
- DH ratchet step ocorre corretamente quando o outro lado responde
- Mensagens fora de ordem (chegada atrasada) decifram via `skipped_keys`
- Ciphertext adulterado é rejeitado (autenticação do AEAD)

**Limite de segurança já implementado**: `MAX_SKIP = 1000` em `ratchet.rs`
— evita que um atacante force acumulação indefinida de chaves puladas em
memória (mitigação de DoS).

**Pendente**: persistência do `RatchetState` em disco (via SQLCipher, ver
seção 4) — atualmente o estado só existe em memória durante o processo.

## 3. Camada de transporte — implementada e testada

Noise Protocol Framework (`Noise_XX_25519_ChaChaPoly_SHA256`) sobre
WebSocket, implementado em `transport/` (crate `qt_transport`),
usando `snow` (Noise) + `tokio-tungstenite` (WebSocket) + `tokio` (runtime
async). Mantém-se agnóstica de conteúdo — só vê bytes já cifrados pelo
Double Ratchet, nunca decifra a camada de aplicação.

Testado de ponta a ponta em `demo/tests/e2e_over_real_websocket.rs`: um
servidor WebSocket real sobe em `localhost`, um cliente liga-se, os dois
fazem o handshake Noise, e uma mensagem já cifrada pelo Double Ratchet
viaja através da ligação — a decifragem em ambas as camadas (Noise e
depois Double Ratchet) é verificada no final.

**Pendente antes de produção:**

- **TLS (`wss://`)**: o transporte atual usa `ws://` puro (sem TLS),
  adequado apenas para testes locais. Em produção, `wss://` deve ser a
  primeira camada de defesa de rede, com o Noise por cima como segunda
  camada de autenticação mútua independente da CA/PKI do TLS.
- **Padrão Noise_IK**: para ligações servidor-servidor de federação, onde a
  chave estática do destino já é conhecida antecipadamente, `Noise_IK`
  permite um handshake com menos round-trips que o `Noise_XX` atual.
- **Pinning de chave estática Noise (lado cliente)**: o servidor já
  persiste a sua própria chave estática entre reinícios (ver secção 4),
  então a base para pinning já existe do lado servidor. `NoiseHandshake::
  remote_static_public_key()` já expõe a chave do par remoto após o
  handshake, mas ainda não há, do lado do CLIENTE, lógica de comparação
  com um "known_hosts" persistido entre sessões — falta isso para o
  pinning ser efetivo, não só possível.
- **Servidor/relay real**: o teste atual simula ambos os lados (cliente e
  "servidor") no mesmo processo de teste. Falta um binário de servidor
  real, com fila de mensagens e lógica de entrega assíncrona (ver secção
  seguinte do roadmap geral do projeto).

## 4. Servidor/relay — implementado e testado

Implementado em `server/` (crate `qt_server`): fila de
mensagens + diretório de pre-keys públicas **persistidos em Redis real**
(não em memória), comunicando através do canal Noise/WebSocket já
validado na secção anterior. As filas têm TTL automático de 30 dias sem
serem levantadas, renovado a cada nova mensagem.

Protocolo de aplicação (`server/src/protocol.rs`, serializado em JSON por
simplicidade nesta fase):
- `RegisterPreKeyBundle` / `FetchPreKeyBundle` — diretório de pre-keys
- `SendMessage { to, sealed_from, ciphertext }` / `FetchMessages` — fila de
  entrega assíncrona. O servidor sabe `to` (precisa disso para rotear),
  mas **já não sabe quem enviou** — `sealed_from` é um envelope selado
  (ver secção 7) que só o destinatário consegue abrir.

Testado em `server/tests/relay_flow.rs` contra um Redis real (não mockado),
incluindo o caso mais importante de um relay: **entrega assíncrona real** —
Alice liga-se, envia, desliga-se; só depois Bob se liga (ligação TCP
separada) e recebe da fila, decifrando corretamente com o Double Ratchet
do lado dele.

**Persistência confirmada manualmente da forma mais convincente possível**:
registado o bundle de Bob → processo do servidor morto com `kill -9` →
confirmado no Redis (`redis-cli KEYS`) que os dados continuavam lá →
subido um servidor completamente novo (chave estática Noise diferente,
novo processo) → Alice conseguiu enviar uma mensagem a Bob sem ninguém se
ter registado outra vez, e Bob recebeu-a corretamente.

**Pendente antes de produção:**

- **Federação**: por agora um único servidor. Protocolo servidor-servidor
  para federação real fica para uma fase posterior.
- **Rate limiting / autenticação de registo**: neste momento qualquer
  ligação pode registar um bundle para qualquer `user_id` — falta lógica
  de autenticação (ex.: provar posse da identity key correspondente antes
  de aceitar um `RegisterPreKeyBundle`).

**Chave estática Noise do servidor — implementada e testada**: já não é
gerada de novo a cada arranque. `server/src/bin/relay_server.rs` carrega-a
de um ficheiro local (`NOISE_KEY_PATH`, por omissão `relay_noise_key.bin`,
permissões `0600`), gerando-a apenas no primeiro arranque. Testado matando
o processo com `kill -9` e arrancando um novo: a chave pública impressa
foi exatamente a mesma nos dois arranques. A função de reconstrução
(`static_keypair_from_private_bytes`, em `transport/src/noise_session.rs`)
deriva a chave pública X25519 a partir dos bytes privados guardados.

Nota de segurança documentada no próprio binário: a chave fica em texto
simples no disco (só protegida por permissões do sistema de ficheiros) —
aceitável para um servidor com acesso físico controlado, mas não é o
nível de proteção usado para chaves de utilizador final (essas usam
SQLCipher, ver secção 5).

## 5. Armazenamento local (no cliente) — implementado e testado

Implementado em `storage/` (crate `qt_storage`): SQLCipher
(ligado à biblioteca do sistema `libsqlcipher`, não uma reimplementação
própria de cifra), guardando:
- A identidade do utilizador (`core::primitives::DhKeyPair` de identidade,
  `SigningKeyPair`, signed pre-key, one-time pre-key)
- Sessões de Double Ratchet por contacto, usando `RatchetState::to_bytes()`/
  `from_bytes()` (adicionado a `core/` especificamente para permitir esta
  persistência sem acoplar `core/` a nenhuma biblioteca de serialização)

Testado em `storage/tests/persistence.rs`: identidade e sessão sobrevivem a
fechar e reabrir a ligação à base de dados; uma passphrase errada não
consegue ler os dados; confirmado manualmente (fora do teste automatizado)
que o ficheiro em disco não contém a assinatura padrão do SQLite nem
nenhum conteúdo legível.

**Integração completa testada**: `cli/src/bin/messenger.rs` liga tudo isto
- `identity`, `register`, `send`, `receive` como comandos completamente
separados (processos reais distintos, não simulados). Testado manualmente:
Bob cria identidade → publica bundle → Alice cria identidade → envia
mensagem (X3DH automático, primeira sessão) → Bob recebe e decifra num
processo separado → segunda mensagem confirma que a sessão persistida
continua corretamente. O wire format da primeira mensagem de uma sessão
inclui um pequeno cabeçalho extra (identity key + ephemeral key do X3DH,
ver `cli/src/bin/messenger.rs` para o formato exato) para o destinatário
conseguir completar o handshake de forma completamente assíncrona.

**Pendente antes de produção:**

- **Origem da passphrase**: atualmente é um argumento de linha de
  comandos. Numa app real, tem de vir do Android Keystore / iOS Secure
  Enclave — nunca um literal no código nem passada por argumento (fica no
  histórico do shell).
- **One-time pre-key reutilizada**: o CLI atual usa sempre a mesma
  one-time pre-key para todos os handshakes recebidos (simplificação
  documentada) — em produção cada uma deve ser consumida uma única vez e
  substituída por uma nova.
- **`PRAGMA secure_delete = ON`** já está ativo (relevante para mensagens
  que desaparecem não deixarem resíduos recuperáveis), mas ainda não há
  lógica de TTL/expiração de mensagens implementada.

## 6. Bindings FFI (Android/iOS/Python) — implementado e testado

Implementado em `ffi/` (crate `qt_ffi`), usando
[uniffi](https://mozilla.github.io/uniffi-rs/) (a mesma ferramenta usada
pelo Signal e por vários projetos Mozilla). Desenho **funcional**
deliberado: todas as funções expostas recebem e devolvem bytes/records
simples, sem objetos com estado mutável partilhado entre a fronteira
FFI — o cliente móvel guarda os bytes de estado (identidade, sessão)
exatamente como o `storage/` já faz no lado desktop, só que no
armazenamento seguro nativo do SO (Keystore/Secure Enclave).

API exposta: geração de chaves DH/assinatura, `sign_signed_pre_key`,
`x3dh_initiate`/`x3dh_respond`, `ratchet_init_as_initiator`/
`init_as_responder`/`encrypt`/`decrypt`, `seal_sender`/`unseal_sender`.

Gerados e testados bindings para **Kotlin**, **Swift**, e **Python**
(`ffi/generate_bindings.sh`). Testado com 7 testes reais a partir do
Python (`ffi/tests/test_ffi_bindings.py`), cobrindo o mesmo que os testes
Rust de `core/` já cobriam, mas atravessando a fronteira FFI: X3DH com
segredos a coincidir entre Alice e Bob, assinatura adulterada rejeitada,
Double Ratchet completo, sealed sender a revelar corretamente o
remetente, e uma chave errada a não conseguir abrir o envelope.

**Pendente antes de produção:**

- **Apps nativas reais**: os bindings gerados (`ffi/bindings/kotlin/`,
  `ffi/bindings/swift/`) ainda não têm nenhuma app Android/iOS à volta
  deles — só foram testados via Python neste ambiente de desenvolvimento.
- **Empacotamento para Android**: falta gerar as bibliotecas nativas
  (`.so`) para cada arquitetura Android (arm64-v8a, armeabi-v7a, x86_64)
  via `cargo-ndk`, e empacotá-las num `.aar`.
- **Empacotamento para iOS**: falta gerar um `XCFramework` a partir da
  biblioteca estática, para consumo direto no Xcode.
- **Camada de transporte/storage no FFI**: o FFI atual só expõe `core/` —
  `transport/` e `storage/` (SQLCipher) ainda vivem só no lado desktop/CLI;
  uma app móvel real também precisaria de bindings para essas camadas (ou
  reimplementar essa parte nativamente, usando bibliotecas equivalentes
  já existentes em Android/iOS para WebSocket e SQLCipher).

## 7. Sealed sender — implementado e testado

Implementado em `core/src/sealed_sender.rs`: cifra anónima (estilo ECIES)
contra a chave pública de identidade do destinatário. O remetente gera um
par de chaves efémero por mensagem, faz DH com a chave pública do
destinatário, deriva uma chave via HKDF, e cifra a sua própria chave de
identidade pública com ChaCha20-Poly1305. Só quem tem a chave privada
correspondente ao destinatário consegue reproduzir o DH e abrir o envelope.

O servidor continua a saber `to` (necessário para rotear a mensagem para a
fila certa) — isto não é anonimato de rede completo (não esconde IP nem
timing), apenas remove a identidade do remetente do protocolo de
aplicação. Anonimato de rede mais forte (ex.: mixnet, Tor) fica fora do
escopo atual.

Testado a três níveis:
- `core/tests/sealed_sender_flow.rs`: destinatário certo abre o envelope;
  qualquer outra chave privada não consegue reconstruir a identidade
  correta; envelope adulterado falha a decifrar (deteção do AEAD).
- `server/tests/relay_flow.rs`: confirma que o campo guardado/entregue
  pela fila não é a string em bruto do remetente, e que só a chave privada
  do destinatário consegue abrir o envelope recebido através do servidor real.
- `cli/src/bin/messenger_demo.rs`: a demo de terminal mostra explicitamente
  "remetente ainda selado" no momento da entrega, e só depois de Bob abrir
  o envelope é que a identidade de Alice é confirmada.

**Pendente antes de produção:**

- **Metadados de timing/tamanho**: sealed sender esconde a identidade do
  remetente, mas não esconde quando uma mensagem foi enviada nem o seu
  tamanho aproximado — um adversário observando o servidor ainda pode
  correlacionar padrões de tráfego. Padding de tamanho fixo e jitter no
  envio são melhorias futuras (ver conversa de arquitetura original).
- **Autenticação de registo**: ver nota na secção 4 — falta impedir que
  alguém publique um bundle a fingir ser outra identidade.

## Próximos documentos a escrever

- `threat-model.md` — modelo de ameaças formal (quem é o adversário, o que
  ele consegue ver em cada camada)
- `architecture-decisions/0001-escolha-x3dh-vs-pqxdh.md`
- `architecture-decisions/0002-uniffi-ffi.md`
