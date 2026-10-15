# Especificação do Protocolo (v0.1: congelado para testes)

> **v0.2 (fase Android).** As secções 8–11 no fim deste ficheiro descrevem os
> formatos e fluxos **atuais** usados pela API `QtClient` e pelo relay
> autenticado; onde divergem das secções 3, 4 e 6 (escritas para a v0.1),
> **prevalecem as secções 8–11**. Mudanças de formato desde a v0.1, cada uma
> motivada por um problema real: (1) o bundle PQXDH passou a ser assinado
> *incluindo a chave ML-KEM* (bundle v2); (2) o relay exige autenticação e
> entrega *at-least-once* com `AckMessages`; (3) `DeliveredMessage` ganhou `id`;
> (4) `RatchetState::decrypt` é transacional (sem mudança de formato).

## Estado deste documento

Rascunho inicial. Cobre o que já está implementado, **compilado e testado**
em `core/`, e o que falta antes de qualquer uso real. Não é ainda uma
especificação formal revisada por pares.

## Nota sobre estabilidade do formato

Os formatos de fio descritos aqui (X3DH/PQXDH, mensagens do Double
Ratchet, envelope do sealed sender, bundle serializado em
`cli/src/wire_format.rs`) estão **congelados nesta versão (v0.1)**
enquanto decorre a bateria de testes de robustez, fuzzing, e qualquer
auditoria futura. Qualquer mudança ao formato de bytes trocados entre
cliente e servidor, ou entre dois clientes, deve:

1. Ser justificada por um problema real encontrado (bug de segurança,
   limitação genuína), não por preferência estética
2. Incluir um número de versão explícito no formato afetado, para que
   clientes antigos e novos consigam, no mínimo, detetar a incompatibilidade
   em vez de falhar de forma confusa
3. Ser registada aqui e no `CHANGELOG.md`

Isto existe para que o trabalho de testes (`docs/threat-model.md`,
`core/fuzz/`) tenha um alvo estável, em vez de estar sempre a testar um
formato que já mudou outra vez.

## Índice

1. [Handshake inicial](#1-handshake-inicial-implementado-e-testado): X3DH e PQXDH
2. [Double Ratchet](#2-double-ratchet-implementado): cifra contínua, forward secrecy
3. [Camada de transporte](#3-camada-de-transporte-implementada-e-testada): Noise sobre WebSocket
4. [Servidor/relay](#4-servidorrelay-implementado-e-testado): fila de mensagens, Redis
5. [Armazenamento local](#5-armazenamento-local-no-cliente-implementado-e-testado): SQLCipher
6. [Bindings FFI](#6-bindings-ffi-androidiospython-implementado-e-testado): Kotlin/Swift/Python
7. [Sealed sender](#7-sealed-sender-implementado-e-testado): esconder o remetente do servidor

*(Se preferires uma explicação sem jargão técnico, começa antes por
[`apresentacao-projeto.md`](../../apresentacao-projeto.md). Este
documento aqui é a referência técnica detalhada, não a porta de entrada.)*

## 1. Handshake inicial: implementado e testado

Duas variantes, ambas em `core/`, ambas com testes de integração a passar:

**X3DH clássico** (`core/src/x3dh.rs`): 3-4x Diffie-Hellman (X25519) via
HKDF-SHA256, com:
- Assinatura Ed25519 da signed pre-key, verificada pelo iniciador antes de
  confiar nela (protege contra um servidor malicioso substituir a chave).
- One-time pre-key opcional (DH4), com degradação graciosa quando o
  servidor não tem nenhuma disponível.
- Testado em `core/tests/full_flow.rs` (4 testes, incluindo casos que
  devem falhar: assinatura adulterada, mensagem adulterada).

**PQXDH** (`core/src/pqxdh.rs`, atrás da feature `pq`): a mesma estrutura,
com um encapsulamento ML-KEM-768 (Kyber, FIPS 203) combinado no HKDF final,
para resistência a "colher agora, decifrar depois" por um adversário com
computador quântico futuro. Exige Rust 1.81+ (dependência `hybrid-array`).
Testado em `core/tests/pqxdh_flow.rs`.

**Pendente**: nenhum dos dois expõe ainda uma API para o servidor consumir
one-time pre-keys de um pool (isso é lógica de servidor, fora do escopo de
`core/`; ver secção 4 do roadmap geral do projeto).

## 2. Double Ratchet: implementado

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
evita que um atacante force acumulação indefinida de chaves puladas em
memória (mitigação de DoS).

**Pendente**: persistência do `RatchetState` em disco (via SQLCipher, ver
seção 4). Atualmente o estado só existe em memória durante o processo.

## 3. Camada de transporte: implementada e testada

Noise Protocol Framework (`Noise_XX_25519_ChaChaPoly_SHA256`) sobre
WebSocket, implementado em `transport/` (crate `qt_transport`),
usando `snow` (Noise) + `tokio-tungstenite` (WebSocket) + `tokio` (runtime
async). Mantém-se agnóstica de conteúdo: só vê bytes já cifrados pelo
Double Ratchet, nunca decifra a camada de aplicação.

Testado de ponta a ponta em `demo/tests/e2e_over_real_websocket.rs`: um
servidor WebSocket real sobe em `localhost`, um cliente liga-se, os dois
fazem o handshake Noise, e uma mensagem já cifrada pelo Double Ratchet
viaja através da ligação, e a decifragem em ambas as camadas (Noise e
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
  com um "known_hosts" persistido entre sessões; falta isso para o
  pinning ser efetivo, não só possível.
- **Servidor/relay real**: o teste atual simula ambos os lados (cliente e
  "servidor") no mesmo processo de teste. Falta um binário de servidor
  real, com fila de mensagens e lógica de entrega assíncrona (ver secção
  seguinte do roadmap geral do projeto).

## 4. Servidor/relay: implementado e testado

Implementado em `server/` (crate `qt_server`): fila de
mensagens + diretório de pre-keys públicas **persistidos em Redis real**
(não em memória), comunicando através do canal Noise/WebSocket já
validado na secção anterior. As filas têm TTL automático de 30 dias sem
serem levantadas, renovado a cada nova mensagem.

Protocolo de aplicação (`server/src/protocol.rs`, serializado em JSON por
simplicidade nesta fase):
- `RegisterPreKeyBundle` / `FetchPreKeyBundle`: diretório de pre-keys
- `SendMessage { to, sealed_from, ciphertext }` / `FetchMessages`: fila de
  entrega assíncrona. O servidor sabe `to` (precisa disso para rotear),
  mas **já não sabe quem enviou**: `sealed_from` é um envelope selado
  (ver secção 7) que só o destinatário consegue abrir.

Testado em `server/tests/relay_flow.rs` contra um Redis real (não mockado),
incluindo o caso mais importante de um relay: **entrega assíncrona real**.
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
  ligação pode registar um bundle para qualquer `user_id`; falta lógica
  de autenticação (ex.: provar posse da identity key correspondente antes
  de aceitar um `RegisterPreKeyBundle`).

**Chave estática Noise do servidor (implementada e testada)**: já não é
gerada de novo a cada arranque. `server/src/bin/relay_server.rs` carrega-a
de um ficheiro local (`NOISE_KEY_PATH`, por omissão `relay_noise_key.bin`,
permissões `0600`), gerando-a apenas no primeiro arranque. Testado matando
o processo com `kill -9` e arrancando um novo: a chave pública impressa
foi exatamente a mesma nos dois arranques. A função de reconstrução
(`static_keypair_from_private_bytes`, em `transport/src/noise_session.rs`)
deriva a chave pública X25519 a partir dos bytes privados guardados.

Nota de segurança documentada no próprio binário: a chave fica em texto
simples no disco (só protegida por permissões do sistema de ficheiros),
aceitável para um servidor com acesso físico controlado, mas não é o
nível de proteção usado para chaves de utilizador final (essas usam
SQLCipher, ver secção 5).

## 5. Armazenamento local (no cliente): implementado e testado

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
  Enclave, nunca um literal no código nem passada por argumento (fica no
  histórico do shell).
- **One-time pre-key reutilizada**: o CLI atual usa sempre a mesma
  one-time pre-key para todos os handshakes recebidos (simplificação
  documentada). Em produção, cada uma deve ser consumida uma única vez e
  substituída por uma nova.
- **`PRAGMA secure_delete = ON`** já está ativo (relevante para mensagens
  que desaparecem não deixarem resíduos recuperáveis), mas ainda não há
  lógica de TTL/expiração de mensagens implementada.

## 6. Bindings FFI (Android/iOS/Python): implementado e testado

Implementado em `ffi/` (crate `qt_ffi`), usando
[uniffi](https://mozilla.github.io/uniffi-rs/) (a mesma ferramenta usada
pelo Signal e por vários projetos Mozilla). Desenho **maioritariamente
funcional**: a generalidade das funções expostas recebem e devolvem
bytes/records simples, sem estado partilhado entre a fronteira FFI: o
cliente móvel guarda os bytes de estado (identidade, sessão) exatamente
como o `storage/` já faz no lado desktop, só que no armazenamento seguro
nativo do SO (Keystore no Android, Secure Enclave no iOS).

**API do núcleo** (funcional, bytes dentro/fora): geração de chaves
DH/assinatura, `sign_signed_pre_key`, `x3dh_initiate`/`x3dh_respond`,
`ratchet_init_as_initiator`/`init_as_responder`/`encrypt`/`decrypt`,
`seal_sender`/`unseal_sender`.

**API de transporte** (`NoiseSession`, objeto com estado): ao contrário do
resto, o handshake Noise é interativo (várias trocas de mensagens), por
isso é exposto como um objeto com estado (`uniffi::Object`) em vez de
funções puras: `new_initiator`/`new_responder`, `write_step`/`read_step`
(o handshake propriamente dito), `is_finished`, e depois de terminado,
`encrypt`/`decrypt` para as mensagens de aplicação. O envio/receção real
dos bytes pela rede continua do lado Kotlin/Swift (usando uma biblioteca
de WebSocket normal, ex.: OkHttp); o `NoiseSession` só trata da parte
criptográfica.

Gerados e testados bindings para **Kotlin**, **Swift**, e **Python**
(`ffi/generate_bindings.sh`). Testado com 7 testes reais a partir do
Python (`ffi/tests/test_ffi_bindings.py`) cobrindo X3DH, Double Ratchet e
sealed sender através da fronteira FFI, e mais um teste dedicado ao
`NoiseSession` (`ffi/tests/test_noise_session.py`): handshake completo
entre duas sessões + troca de mensagens cifradas nos dois sentidos.

**Testado também num Android real** (emulador): uma app mínima em Kotlin,
consumindo os bindings gerados, correu o fluxo completo (geração de
identidade, X3DH, Double Ratchet, sealed sender), tudo dentro do
telemóvel, com resultado visível no ecrã.

**Pendente antes de produção:**

- **App Android/iOS completa**: o que existe é uma demo de um ecrã (prova
  que o FFI funciona no dispositivo), não ainda uma app com ecrã de
  conversa, lista de contactos, ou ligação de rede real a um servidor.
- **Empacotamento para Android**: as bibliotecas nativas (`.so`) já são
  geradas via `cargo-ndk` para arm64-v8a/armeabi-v7a/x86_64, mas ainda não
  empacotadas num `.aar` publicável.
- **Empacotamento para iOS**: falta gerar um `XCFramework` a partir da
  biblioteca estática, para consumo direto no Xcode (ainda não testado em
  iOS, só Android).
- **`storage/` (SQLCipher) não é exposto via FFI**, de propósito: uma
  app móvel real deve usar o armazenamento seguro nativo do próprio SO
  (Keystore/EncryptedSharedPreferences no Android, Secure Enclave no iOS),
  não SQLCipher cross-compilado, que traria complexidade extra sem
  necessidade.

## 7. Sealed sender: implementado e testado

Implementado em `core/src/sealed_sender.rs`: cifra anónima (estilo ECIES)
contra a chave pública de identidade do destinatário. O remetente gera um
par de chaves efémero por mensagem, faz DH com a chave pública do
destinatário, deriva uma chave via HKDF, e cifra a sua própria chave de
identidade pública com ChaCha20-Poly1305. Só quem tem a chave privada
correspondente ao destinatário consegue reproduzir o DH e abrir o envelope.

O servidor continua a saber `to` (necessário para rotear a mensagem para a
fila certa). Isto não é anonimato de rede completo (não esconde IP nem
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
  tamanho aproximado: um adversário observando o servidor ainda pode
  correlacionar padrões de tráfego. Padding de tamanho fixo e jitter no
  envio são melhorias futuras (ver conversa de arquitetura original).
- **Autenticação de registo**: ver nota na secção 4; falta impedir que
  alguém publique um bundle a fingir ser outra identidade.

## 8. Qual fluxo estabelece a sessão? X3DH vs PQXDH vs Double Ratchet (v0.2)

Três peças, em camadas:

1. **X3DH** (`core/src/x3dh.rs`): acordo de chave clássico (X25519). Mantido
   no core, usado pela API legacy e pela CLI de demonstração. **Não** é usado
   pela API `QtClient`.
2. **PQXDH** (`core/src/pqxdh.rs`, feature `pq`): **é o mecanismo de
   estabelecimento de sessão da API `QtClient`/Android.** Igual ao X3DH mais
   um encapsulamento ML-KEM-768:
   ```
   DH1 = X25519(IK_A_priv, SPK_B)      DH2 = X25519(EK_A, IK_B)
   DH3 = X25519(EK_A,       SPK_B)     DH4 = X25519(EK_A, OPK_B)   (opcional)
   (ct, ss_pq) = ML-KEM-768.Encaps(PQPK_B)
   SK = HKDF-SHA256(ikm = DH1‖DH2‖DH3[‖DH4]‖ss_pq, salt = vazio, info = "PQXDH_v1", 32 bytes)
   ```
   Bob calcula o mesmo `SK` com as chaves privadas e `Decaps(ct)`. O iniciador
   só prossegue se a assinatura Ed25519 do bundle for válida (ver §9).
3. **Double Ratchet** (`core/src/ratchet.rs`): `SK` é a root key inicial
   (Alice: `init_as_initiator(SK, SPK_B)`; Bob: `init_as_responder(SK, SPK_B_keypair)`).
   Daí em diante cada mensagem usa uma chave de mensagem de uso único
   (ChaCha20-Poly1305, AAD = chave DH ‖ n).

Fluxo ponta a ponta da app:
```
Bob:   createIdentity → publishableBundle → relay.register
Alice: (QT Public ID de Bob) → relay.fetchBundle → startSession   # verifica + PQXDH + ratchet
Alice: encryptSealed → relay.send            # 1.as mensagens levam o cabeçalho PQXDH
Bob:   relay.fetch → decryptSealed → (sessão criada) → relay.ack
```
Propriedades PQ exatas: ver `threat-model.md` §3.1 (híbrido para
confidencialidade do segredo inicial; autenticação, cura e transporte são clássicos).

## 9. Bundle de chaves públicas (v3)

Binário (1349 bytes), usado entre cliente e relay:
`[3][IK 32][IK_sign 32][SPK 32][sig 64][pq_prekey_id u32 BE][ML-KEM-768 pub 1184]`.
`sig = Ed25519(IK_sign, "QT-PQXDH-v2-bundle" ‖ IK ‖ SPK ‖ ML-KEM pub)`. A
assinatura cobre a identidade X25519 (`IK`), por isso a chave Ed25519 **atesta**
a X25519 e nenhuma das duas pode ser trocada isoladamente.

* **Relay:** só aceita `RegisterPreKeyBundle` se `user_id` = utilizador
  autenticado, `IK_sign` do bundle = a chave Ed25519 autenticada na ligação e a
  assinatura validar. Ninguém escreve no slot de outro, nem regista um bundle
  que reclame a identidade de outro.
* **Cliente:** verifica `IK`/`IK_sign` iguais aos do QT Public ID, assinatura
  válida, chave ML-KEM canónica.
* **Safety number:** 60 dígitos (12×5) derivados de `IK ‖ IK_sign` de ambos
  (SHA-512 iterado 5200×, ordenados para serem simétricos). Mostrado na app
  para comparação fora de banda. Não inclui one-time prekeys.

**QT Public ID:** `"qt1:" + hex(IK ‖ IK_sign)` (64 bytes → 128 hex).
**contactId:** `hex(IK)`. **Caixa no relay (`user_id`):** `hex(IK_sign)`.

## 10. Envelope v1 (o que o relay transporta)

```
Envelope      = [1: versão=2][1: tipo][se tipo=1: Cabeçalho][Mensagem de ratchet]
tipo          = 0 mensagem normal | 1 mensagem com cabeçalho PQXDH
Cabeçalho     = [32: EK_A pública][4: opk_id BE (0 = nenhuma)][4: pq_prekey_id BE][1088: ct ML-KEM]
Mensagem      = [32: chave DH do ratchet][4: PN BE][4: n BE][N: nonce(12) ‖ AEAD ct ‖ tag(16)]
Selado        = [1: versão=2][2: len BE][caixa selada (identidade do remetente)][Envelope]
Caixa selada  = [32: efémera pública][nonce ‖ AEAD(IK_remetente, AAD = efémera ‖ SHA-256("QT-SEALED-BIND-v1" ‖ Envelope))]  (chave = HKDF(X25519(efémera, IK_destinatário)))
```
* O cabeçalho **não** contém a identidade do remetente (vai na caixa selada,
  ou é indicada pelo relay no modo não selado).
* Enquanto o iniciador não receber nada do contacto, **todas** as mensagens
  levam o cabeçalho (a 1.ª pode chegar fora de ordem ou perder-se).
* **Remetente autenticado pela sessão:** a identidade na caixa selada é só uma
  *pista* para escolher a sessão. O AAD de cada mensagem do ratchet inclui
  `"QT-AD-v1" ‖ IK_iniciador ‖ IK_respondedor ‖ chave DH ‖ PN ‖ n`; só quem
  tem as chaves da sessão (e as duas identidades certas) autentica. A caixa
  está ligada ao envelope (hash no AAD) e não pode ser reutilizada com outro.
* **Ratchet:** `PN` (tamanho da cadeia de envio anterior) permite decifrar
  mensagens atrasadas da cadeia anterior após um DH ratchet. Salto máximo por
  cadeia: 1000; limite global de chaves puladas guardadas: 2000 (as mais
  antigas são descartadas). `decrypt` trabalha numa cópia do estado e só a
  confirma se o AEAD autenticar.
* ID do envelope para deduplicação = `(chave DH ‖ n)`; registado **depois** de
  decifrar com sucesso (para um forjador não poder “envenenar” a lista).
* O destinatário só consome a one-time prekey **depois** de a 1.ª mensagem
  autenticar. Falhas nunca alteram o estado persistido.
* Limite de plaintext: 10 000 bytes.

## 11. Protocolo do relay (v0.2)

Transporte: WebSocket (`wss://` em produção) → Noise `XX_25519_ChaChaPoly_SHA256`
→ mensagens JSON (`serde`, enum externamente etiquetado) cifradas pelo Noise.
Um pedido ↔ uma resposta, sequenciais.

1. **Autenticação obrigatória (1.ª mensagem):**
   `Authenticate{signing_public, signature}` com
   `signature = Ed25519(IK_sign, "QT-RELAY-AUTH-v1" ‖ hash_do_handshake_Noise)`.
   O `user_id` passa a ser `hex(signing_public)`. Sem autenticação válida → erro
   e fecho. A assinatura liga-se à sessão (channel binding): não é reutilizável.
2. `RegisterPreKeyBundle{user_id, bundle_bytes}`: só para o próprio `user_id`; ≤ 24 000 bytes.
3. `FetchPreKeyBundle{user_id}`: qualquer utilizador autenticado.
4. `SendMessage{to, sealed_from, ciphertext}`: `to` = 64 hex minúsculos;
   `ciphertext` 1…14 000 bytes; `sealed_from` ≤ 1024 (vazio no modo `QtClient`,
   que envia o blob selado inteiro em `ciphertext`); fila ≤ 1000 mensagens.
5. `FetchMessages{user_id}`: só a própria fila; **não apaga**; devolve uma
   página (≈ 40 kB de JSON) de `{id, sealed_from, ciphertext}`.
6. `AckMessages{user_id, ids}`: só a própria fila; ≤ 200 ids; apaga as confirmadas.

7. **Anexos** (ver secção 14), com o ficheiro JÁ cifrado pelo cliente:
   `BlobCreate{size}` (≤ 27 MiB) → `BlobCreated{blob_id, token}`;
   `BlobPut{blob_id, token, offset, data}` (blocos de 40 KiB em base64, por ordem;
   repetir um bloco já recebido é aceite) → `BlobStatus{size, received, complete}`;
   `BlobInfo{blob_id}` → `BlobStatus` (para retomar); `BlobGet{blob_id, offset}` →
   `BlobData{offset, size, data}` (só ficheiros completos); `BlobDelete{blob_id}` → `Ack`.
   Ficheiro inexistente ou expirado → `BlobNotFound`.

Respostas: `Ack`, `PreKeyBundle`, `PreKeyBundleNotFound`, `Messages`, `Error{reason}`
(`reason` genérico, nunca detalhes internos), e as dos anexos acima.
O cliente pode enviar vários pedidos seguidos antes de ler as respostas: o relay
responde pela mesma ordem (as apps usam isto para os blocos dos anexos).
Timeouts do servidor (omissão): handshake 10 s, autenticação 10 s, inatividade 300 s.
Qualquer frame Noise inválido/repetido/fora de ordem encerra a sessão.
Entrega *at-least-once*: o cliente tem de deduplicar (o `QtClient` faz-o).

## 12. Mensagem dentro do ratchet (payload v1) e estados

O plaintext cifrado pelo Double Ratchet tem agora um formato próprio (o relay nunca o vê):

```
[1 versão=0x01][1 tipo][16 msg_id][8 sent_at ms, BE]
tipo 1 = texto:  [texto UTF-8, máx. 9000 bytes]
tipo 2 = recibo: [1 estado: 2=entregue, 3=vista][2 n BE][n × 16 msg_id], n ≤ 400
tipo 3 = anexo:  [1 versão=1][32 chave][16 blob_id][8 tamanho BE][8 tamanho cifrado BE]
                 [4 largura BE][4 altura BE][1 n][n bytes do tipo MIME]
```

Um plaintext fora deste formato (clientes antigos) é tratado como texto simples, com `msg_id` derivado do envelope.

**Estados** de uma mensagem enviada: *por enviar* (na outbox) → *enviada* (o relay deu `Ack`) → *entregue* (recibo cifrado do destinatário) → *vista* (recibo cifrado, opcional). Os estados só sobem e um recibo só afeta mensagens enviadas ao próprio remetente do recibo.

**Atomicidade** (`ffi/src/history.rs`): enviar = avançar o ratchet + guardar a mensagem + pôr o envelope na outbox, numa transação; receber = avançar o ratchet + marcar o envelope como visto + guardar a mensagem/aplicar o recibo + agendar o recibo "entregue", numa transação. A outbox só é limpa quando o relay confirma. Testes: `ffi/tests/history_flow.rs`.

## 13. Sessoes cruzadas, reinicio de sessao e eliminacao

**Sessoes cruzadas.** Se A e B iniciam uma sessao PQXDH um com o outro ao mesmo
tempo, cada um recebe uma mensagem com cabecalho PQXDH (tipo 1) que a sua
sessao nao decifra. Desempate deterministico, igual nos dois lados: fica a
sessao iniciada por quem tem a chave de identidade X25519 *menor* (comparacao
byte a byte). Quem perde adota a sessao do outro e volta a cifrar, com a sessao
nova, as mensagens que ainda nao tinham recibo "entregue" (mesmo `msg_id`, por
isso o destinatario nunca as mostra em duplicado). Quem ganha descarta a
mensagem do outro (ela volta a chegar, ja na sessao certa).

**Reinicio de sessao.** Com uma sessao estabelecida, uma mensagem tipo 1 que a
sessao atual nao decifra, mas que abre uma sessao nova valida a partir do seu
cabecalho, substitui a sessao (o contacto apagou-a e voltou a escrever). So o
dono da chave privada de identidade do contacto consegue produzir isto (o
PQXDH usa-a e o AEAD autentica as duas identidades). Um cabecalho com a mesma
chave efemera que criou a sessao atual nunca a substitui (seria voltar atras).

**Eliminacao.** *Eliminar conversa* apaga mensagens e outbox do contacto (a
sessao mantem-se). *Apagar contacto* apaga tambem a sessao, os metadados e os
registos anti-repeticao, que sao trocados por `SHA-256("QT-SEEN-TOMBSTONE-v1"
|| len || contact_id || envelope_id)`: um envelope antigo reentregue pelo relay
continua a ser reconhecido, sem o contacto ficar em claro. Tudo com
`secure_delete` (paginas apagadas sobrescritas).

**Tipos desconhecidos.** Um payload v1 com tipo que a versao nao conhece e
guardado como aviso ("atualiza a app"), nunca como texto.

## 14. Anexos: imagens e GIFs (ate 25 MiB)

Implementado em `ffi/src/attachments.rs` (cliente) e `server/src/blobs.rs` (relay).
Testes: `server/tests/attachments_e2e.rs` e `server/tests/relay_blobs.rs`.

**Cifra do ficheiro.** Cada anexo tem uma chave nova de 32 bytes (CSPRNG). O
conteudo e arredondado para cima com Padme (no maximo ~12% a mais, preenchido
com zeros) e cifrado em blocos de 40 944 bytes com ChaCha20-Poly1305, AAD
`QT-ATT-v1` e nonce = `[8 numero do bloco BE][3 zeros][1 ultimo bloco? 1 : 0]`.
Cada bloco cifrado tem 40 KiB, o tamanho de um `BlobPut`. Trocar, repetir,
reordenar ou cortar blocos faz a verificacao falhar; o recetor so marca o
anexo como pronto depois de verificar o ficheiro inteiro. Tipos aceites:
`image/jpeg`, `image/png`, `image/gif`, `image/webp`.

**Envio.** O cliente guarda o ficheiro cifrado, reserva espaco no relay, envia
os blocos e so depois manda, pelo Double Ratchet (com Sealed Sender), a
mensagem tipo 3 com a chave, o `blob_id`, os tamanhos e o tipo. O `token` de
escrita nunca sai do telemovel de quem envia.

**Rececao.** A mensagem tipo 3 entra no historico como "a descarregar"; o
cliente descarrega os blocos, verifica tudo e pede ao relay `BlobDelete`.

**No relay.** Bytes opacos num ficheiro por anexo (`anexos/<blob_id>`, 0600),
registo no Redis com TTL de 24 h (`size`, hash do token, completo/incompleto) e
limpeza do disco de 10 em 10 minutos. Nao fica registado quem criou cada
ficheiro. Limites: 27 MiB por ficheiro, espaco total configuravel, espaco minimo livre
no disco (1 GiB por omissao) e bytes enviados por utilizador por dia (contador `blobquota:<user>:<dia>`, 2 dias).

**No telemovel.** Os ficheiros ficam cifrados em `<base de dados>.anexos/`, com
nomes derivados de `SHA-256("QT-ATT-FILE" || contacto || msg_id)`; a chave fica
no SQLCipher. A imagem em claro so existe em memoria, para o ecra. Eliminar a
conversa, o contacto ou todos os dados apaga os ficheiros.

## Documentos relacionados

- [`threat-model.md`](threat-model.md): modelo de ameaças formal (quem
  é o adversário, o que está protegido e o que **não** está); leitura
  obrigatória antes de avaliar este projeto tecnicamente
- [`../ROADMAP.md`](../ROADMAP.md): o plano, por fases
- [`../CHANGELOG.md`](../CHANGELOG.md): histórico do que já mudou

Ainda por escrever: `architecture-decisions/0001-escolha-x3dh-vs-pqxdh.md`,
`architecture-decisions/0002-uniffi-ffi.md` (registos formais de decisões
de arquitetura; por agora, essas decisões estão documentadas de forma
menos formal ao longo das secções acima).
