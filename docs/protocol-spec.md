# Especificação do Protocolo (v0.1: congelado para testes)

> **Formatos atuais.** As secções 8–14 no fim deste ficheiro descrevem os
> formatos e fluxos **atuais** usados pela API `QtClient`, pelas apps e pelo
> relay autenticado; onde divergem das secções 3, 4 e 6 (escritas para a v0.1),
> **prevalecem as secções 8–14**. Mudanças de formato desde a v0.1, cada uma
> motivada por um problema real: (1) o bundle PQXDH passou a ser assinado
> *incluindo a chave ML-KEM* (bundle v2) e depois também a identidade X25519
> (bundle v3); (2) o relay exige autenticação e entrega *at-least-once* com
> `AckMessages`; (3) `DeliveredMessage` ganhou `id`; (4) `RatchetState::decrypt`
> é transacional (sem mudança de formato); (5) envelope v2, com `PN` no
> cabeçalho do ratchet e a caixa selada ligada ao envelope; (6) anexos (secção 14).

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
8. a 18. Formatos atuais (no fim do ficheiro): qual fluxo estabelece a sessão, bundle v3,
   envelope v2, protocolo do relay, mensagem dentro do ratchet, sessões cruzadas e
   eliminação, anexos (imagens, GIFs, voz e ficheiros), grupos, presença, notificações,
   chamadas de voz, foto de perfil

*(Se preferires uma explicação sem jargão técnico, começa antes por
o [`README.md`](../README.md). Este
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
   T  = SHA-256("QT-PQXDH-v2-transcript" ‖ IK_A ‖ IK_B ‖ SPK_B ‖ EK_A ‖ (OPK_B ou 32 zeros) ‖ PQPK_B ‖ ct)
   SK = HKDF-SHA256(ikm = 0xFF×32 ‖ DH1‖DH2‖DH3[‖DH4]‖ss_pq, salt = 0x00×32, info = "QT-PQXDH-v2" ‖ T, 32 bytes)
   ```
   Bob calcula o mesmo `SK` com as chaves privadas e `Decaps(ct)`. O iniciador
   só prossegue se a assinatura Ed25519 do bundle for válida e o bundle tiver
   menos de 90 dias (ver §9). **PQXDH v2, desde a app 0.9.3:** a derivação segue a
   especificação PQXDH do Signal e liga o segredo a todas as chaves públicas e ao
   ciphertext ML-KEM usados (transcrição `T`). O v1 (`ikm` sem o prefixo 0xFF, salt
   vazio, info `PQXDH_v1`, sem transcrição) só é usado para receber primeiras
   mensagens de apps antigas ou de sessões iniciadas antes da 0.9.3.
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

## 9. Bundle de chaves públicas (v4, desde a app 0.9.3)

Binário (1361 bytes):
`[4][IK 32][IK_sign 32][spk_id u32 BE][SPK 32][criado_em u64 BE][sig 64][pq_prekey_id u32 BE][ML-KEM-768 pub 1184]`,
com `sig = Ed25519(IK_sign, "QT-PQXDH-v4-bundle" ‖ IK ‖ spk_id ‖ SPK ‖ criado_em ‖ pq_prekey_id ‖ ML-KEM pub)`.
`criado_em` é a hora (segundos unix) em que a signed pre-key foi criada.

* **Rotação:** ao publicar (sempre que a app abre), se a signed pre-key tiver mais de
  7 dias, o telemóvel cria uma nova (`spk_id` seguinte) e uma prekey ML-KEM nova. As 5
  anteriores de cada ficam guardadas (cerca de 35 dias) para primeiras mensagens ainda
  a caminho; o cabeçalho PQXDH v2 diz qual foi usada (§10).
* **Validade:** quem inicia recusa um bundle com mais de 90 dias ou com data mais de
  um dia no futuro. Um relay não consegue servir uma signed pre-key velha como
  recente (a data está assinada).
* **One-time prekeys:** não estão no bundle. Uma vez por dia o telemóvel envia um
  lote de 100 no `RegisterPreKeyBundle` (§11); o relay entrega uma, e apaga-a, a cada
  `FetchPreKeyBundle`. As privadas ficam no telemóvel até serem usadas ou até 35 dias
  depois do lote. Não são assinadas (como no Signal): um relay que as troque só
  consegue que a sessão comece sem essa proteção extra.

O relay ainda aceita o **bundle v3** (1349 bytes, apps até à 0.9.2), descrito a seguir;
as apps 0.9.3 já não iniciam sessões com ele (o contacto tem de atualizar).

Binário v3 (1349 bytes):
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

## 10. Envelope v3 (o que o relay transporta, desde a app 0.9.3)

```
Envelope v3   = [1: versão=3][1: tipo][se tipo=2: Cabeçalho v2][Mensagem de ratchet]
tipo          = 0 mensagem normal | 2 mensagem com cabeçalho PQXDH v2
Cabeçalho v2  = [32: EK_A pública][4: opk_id BE (0 = nenhuma)][4: pq_prekey_id BE][4: spk_id BE][1088: ct ML-KEM]
```
* **Enchimento:** antes de cifrar, o plaintext leva um byte `0x80` e zeros até ao
  primeiro destes tamanhos que caiba: 256, 1024, 4096 ou 10 240 bytes. Quem vê o
  tamanho dos envelopes (o relay) só distingue esses 4 tamanhos: um recibo, uma
  presença e uma mensagem curta ficam iguais. Quem recebe retira o enchimento
  depois de decifrar; um formato errado conta como envelope inválido.
* O resto (mensagem de ratchet, AD, deduplicação, envelope selado) é igual ao v2
  abaixo. A caixa selada continua com versão 2.
* As apps 0.9.3 ainda **recebem** o v2 (tipos 0 e 1, sem enchimento), e enviam-no só
  para a primeira mensagem de uma sessão iniciada antes da atualização.

### Envelope v2 (apps até à 0.9.2)

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
2. `RegisterPreKeyBundle{user_id, bundle_bytes, pow?, one_time_pre_keys?}`: só para o próprio
   `user_id`; ≤ 24 000 bytes. **Conta nova** (o relay ainda não tem bundle dela): exige `pow`,
   um `nonce` tal que `Argon2id(m = 8 MiB, t = 1, p = 1, "QT-POW-v1" ‖ signing_public ‖ nonce)`
   comece por 7 bits a zero (`core/src/pow.rs`; uns segundos num telemóvel, uma vez por conta).
   `one_time_pre_keys` (até 200, `[4 número BE][32 pública]` cada) substitui as guardadas.
   O bundle expira ao fim de 90 dias; as apps voltam a publicá-lo sempre que abrem
   (arranque, desbloqueio ou ID novo), por isso só expira para quem não abre a app há 90 dias.
3. `FetchPreKeyBundle{user_id}`: qualquer utilizador autenticado. A resposta
   `PreKeyBundle{bundle_bytes, one_time_pre_key?}` leva uma one-time prekey do dono (já
   apagada do relay), se ainda houver. Limite: 60 pedidos seguidos e 60 por minuto por conta.
4. `SendMessage{to, sealed_from, ciphertext, silent, wake}`: `to` = 64 hex minúsculos;
   `ciphertext` 1…14 000 bytes; `sealed_from` ≤ 1024 (vazio no modo `QtClient`,
   que envia o blob selado inteiro em `ciphertext`); fila ≤ 1000 mensagens.
   `silent` (opcional, `false` se faltar): entregue igual, mas sem campainha (secção 17).
   `wake` (opcional, desde a app 0.9.6): `"Call"` ou `"CallEnd"`, campainha marcada de uma
   chamada de voz (secção 17).
   Limite: 300 envelopes seguidos e 300 por minuto por conta (`Error{"demasiados pedidos…"}`;
   as apps deixam o envelope na outbox e voltam a tentar).
5. `FetchMessages{user_id}`: só a própria fila; **não apaga**; devolve uma
   página (≈ 40 kB de JSON) de `{id, sealed_from, ciphertext}`.
   `WaitMessages{user_id, timeout_secs}`: igual a `FetchMessages`, mas, com a
   fila vazia, a resposta fica pendente até chegar uma mensagem para este
   utilizador ou até `timeout_secs` (no máximo 25 s); depois devolve `Messages`
   (vazia se o tempo acabou). É a **entrega imediata**: as apps usam uma ligação
   só para isto (fica ocupada durante a espera) e voltam logo a pedir. Um relay
   anterior a este pedido responde `Error{"mensagem invalida"}`; as apps passam
   então a `FetchMessages` de 3 em 3 s.
6. `AckMessages{user_id, ids}`: só a própria fila; ≤ 200 ids; apaga as confirmadas.
   `DeleteAccount{user_id}`: só a própria conta. Apaga o bundle, a fila e a contagem
   diária de anexos (os anexos não estão ligados a quem os enviou e apagam-se em 24 h).
   Durante 30 dias o relay lembra-se só de que este `user_id` foi apagado: um
   `SendMessage` para ele recebe `Error{"o destinatario apagou a conta"}`. Publicar
   um bundle de novo desfaz isto. Relays antigos respondem `Error{"mensagem invalida"}`.

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
Timeouts do servidor (omissão): handshake 5 s, autenticação 5 s, inatividade 300 s.
Limites (omissão): no máximo 2 000 ligações por autenticar no relay inteiro (as seguintes são
fechadas logo) e 8 ligações autenticadas por conta: uma 9.ª fecha a mais antiga dessa conta.
À frente do relay, o HAProxy limita cada IP a 100 ligações abertas e 100 novas a cada 10 s.
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
                 ficheiro: [1 versão=2] e o mesmo, mais [1 k][k bytes do nome, UTF-8]
tipo 4 = convite: [Public ID de quem escreve, UTF-8]
tipo 5 = reação: [16 msg_id da mensagem][emoji UTF-8, máx. 32 bytes; vazio = retirar]
tipo 6 = mensagem de grupo: [16 group_id][payload v1 completo: tipo 1, 3 ou 5]
tipo 7 = info do grupo:     [16 group_id][JSON: versão, nome, membros e administradores]
tipo 8 = saída do grupo:    [16 group_id]
tipo 9 = presença: [1 estado: 1 = online, 0 = offline]  (msg_id sempre o mesmo)
tipo 10 = sessão terminada: [vazio]
tipo 11 = chamada de voz (ver secção 18)
tipo 12 = foto de perfil: [1 op: 1 = foto, 0 = sem foto][foto WebP ou JPEG, máx. 9 973 bytes]
          (msg_id sempre o mesmo)
```

**Mensagens de voz** são anexos (tipo 3) com o MIME `audio/mp4` (AAC), até 10 s e 1 MiB;
a "largura" leva a duração em ms e a "altura" é 0.

**Ficheiros** (desde a app 0.9.0) são anexos (tipo 3) na versão 2 do corpo: qualquer MIME
`tipo/subtipo` em minúsculas (até 100 bytes, sem parâmetros) e o nome do ficheiro (1 a 200
bytes UTF-8, sem `/`, `\` nem caracteres de controlo, nem `.` ou `..`); largura e altura 0.
Imagens e voz continuam na versão 1, que as versões antigas conhecem. Uma versão antiga
recebe um ficheiro como "atualiza a app".

**Convite** (tipo 4): vai uma vez, antes da primeira mensagem a quem ainda não nos escreveu.
O destinatário só aceita o Public ID se a chave de identidade dele for a da sessão que o
trouxe. Com ele, a app mostra "Aceitar / Recusar / Bloquear" sem pedir o ID à mão.

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
isso o destinatario nunca as mostra em duplicado). Os envelopes de grupos que
ainda estavam na outbox tambem sao cifrados de novo. Quem ganha le a mensagem
do outro na mesma (o AEAD da sessao dele autentica-a), mas fica com a sua
sessao; o outro adota-a quando receber a primeira mensagem dela. Isto acontece
sobretudo nos grupos: dois membros que nao se conheciam abrem sessao um com o
outro ao mesmo tempo.

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

**Sessao terminada (tipo 10, desde a app 0.9.0 de 18/10/2026).** Sem aviso, quem foi
apagado continuava a cifrar com a sessao antiga: a app de quem apagou nao tinha chaves
(`UnknownSession`), confirmava e descartava, e as mensagens perdiam-se em silencio.
Agora, antes de apagar as chaves (`delete_contact_notify`, tambem ao recusar um
convite), a app cifra um tipo 10 com a sessao que vai apagar e deixa-o na outbox com o
endereco do contacto no relay (ele ja nao esta nos contactos). Bloqueado: nao avisa.
Quem recebe (autenticado pelo AEAD da sessao) apaga a sessao dele, fica com o contacto
e o historico, e retem a outbox desse contacto (`restart:`). A app abre logo uma
sessao nova (`sessions_to_restart` + `start_session`): o que estava por entregar
(mensagens sem recibo "entregue" e envelopes de grupo) volta a ser cifrado com a sessao
nova, seguido do convite (tipo 4). Do lado de quem apagou, chega como convite. Se a
sessao nova vier do outro lado primeiro, o reenvio acontece ao recebe-la. Uma versao
antiga mostra o tipo 10 como "atualiza a app". Testes: `ffi/tests/session_reset.rs`.

**Tipos desconhecidos.** Um payload v1 com tipo que a versao nao conhece e
guardado como aviso ("atualiza a app"), nunca como texto.

## 14. Anexos: imagens, GIFs, voz e ficheiros (ate 25 MiB)

Implementado em `ffi/src/attachments.rs` (cliente) e `server/src/blobs.rs` (relay).
Testes: `server/tests/attachments_e2e.rs` e `server/tests/relay_blobs.rs`.

**Cifra do ficheiro.** Cada anexo tem uma chave nova de 32 bytes (CSPRNG). O
conteudo e arredondado para cima com Padme (no maximo ~12% a mais, preenchido
com zeros) e cifrado em blocos de 40 944 bytes com ChaCha20-Poly1305, AAD
`QT-ATT-v1` e nonce = `[8 numero do bloco BE][3 zeros][1 ultimo bloco? 1 : 0]`.
Cada bloco cifrado tem 40 KiB, o tamanho de um `BlobPut`. Trocar, repetir,
reordenar ou cortar blocos faz a verificacao falhar; o recetor so marca o
anexo como pronto depois de verificar o ficheiro inteiro. Tipos aceites:
`image/jpeg`, `image/png`, `image/gif`, `image/webp`, `audio/mp4` (voz) e, nos
ficheiros (versao 2, com nome), qualquer tipo bem formado.

**Envio.** O cliente guarda o ficheiro cifrado, reserva espaco no relay, envia
os blocos e so depois manda, pelo Double Ratchet (com Sealed Sender), a
mensagem tipo 3 com a chave, o `blob_id`, os tamanhos e o tipo. O `token` de
escrita nunca sai do telemovel de quem envia.

**Rececao.** A mensagem tipo 3 entra no historico como "a descarregar"; o
cliente descarrega os blocos, verifica tudo e pede ao relay `BlobDelete` (so nas
conversas a dois; nos grupos o relay apaga-o ao fim de 24 h, ver secao 15).

**No relay.** Bytes opacos num ficheiro por anexo (`anexos/<blob_id>`, 0600),
registo no Redis com TTL de 24 h (`size`, hash do token, completo/incompleto) e
limpeza do disco de 10 em 10 minutos. Nao fica registado quem criou cada
ficheiro. Limites: 27 MiB por ficheiro, espaco total configuravel, espaco minimo livre
no disco (1 GiB por omissao) e bytes enviados por utilizador por dia (contador `blobquota:<user>:<dia>`, 2 dias).

**No telemovel.** Os ficheiros ficam cifrados em `<base de dados>.anexos/`, com
nomes derivados de `SHA-256("QT-ATT-FILE" || conversa || msg_id)`; a chave fica
no SQLCipher. A imagem em claro so existe em memoria, para o ecra. Um ficheiro so
e decifrado a pedido: para outra app o ler da memoria (Android: `SealerFileProvider`,
sem copia em disco) ou para o guardar onde o utilizador escolher. Eliminar a
conversa, o contacto, o grupo ou todos os dados apaga os ficheiros.

## 15. Grupos (até 10 pessoas, 3 administradores)

**Envio a cada membro.** Cada mensagem de grupo é cifrada uma vez para cada membro, pelas
sessões 1 a 1 (PQXDH + Double Ratchet + Sealed Sender). O relay não muda e não sabe que
existe um grupo: vê envelopes 1 a 1 como os outros (no máximo 9 por mensagem).

**Info do grupo** (tipo 7): `{"v": versão, "name": nome, "members": [{"pid": Public ID,
"admin": bool}]}`. Válida com 1 a 10 membros, 1 a 3 administradores, sem repetidos e nome
até 100 bytes. Um grupo desconhecido só é aceite se a info vier de um administrador que ela
própria nomeia e se eu estiver nos membros. Num grupo conhecido, a info tem de vir de quem
já era administrador e com versão maior (uma versão sem ele próprio é um administrador a
sair). Quem é removido recebe a versão nova e fica com o grupo inativo.

**Mensagens** (tipo 6) só são aceites de membros. Uma que chegue antes da info (envelopes
de pessoas diferentes não chegam por ordem) fica guardada até a info chegar (no máximo 100).
Vão texto, reações e, desde a app 0.9.0, anexos (fotos, GIFs, voz e ficheiros); outros tipos
aparecem como "atualiza a app".

**Anexos no grupo.** O ficheiro cifrado vai uma só vez para o relay; a mensagem tipo 3 (com a
chave e o `blob_id`) vai dentro do tipo 6 a cada membro (`group_attachment_uploaded`). Quem
recebe descarrega e verifica como numa conversa a dois, mas não pede `BlobDelete`: os outros
membros ainda o podem estar a descarregar. O relay apaga-o ao fim de 24 horas.

**Sair** (tipo 8): os administradores que o recebem publicam logo uma versão sem essa
pessoa. Um administrador que sai publica ele a versão nova; se era o único, passa a sê-lo o
primeiro membro que fica.

**Estados:** sem recibos de entregue e vista nos grupos. A mensagem fica "enviada" quando o
relay confirmou os envelopes de todos os membros. Na base local, a conversa do grupo tem o
id `g:` + group_id e cada mensagem recebida guarda quem a escreveu. Testes: `ffi/tests/groups.rs`.

## 16. Presença (online / offline)

Só para contactos e cifrada (tipo 9, dentro do Double Ratchet): o relay vê um envelope 1 a 1
como os outros. Nada é guardado em disco; o estado fica em memória (`ffi/src/presence.rs`).

* A app diz ao núcleo quem são os contactos (`presence_configure`). Só desses se aceita a
  presença, e só a esses se envia. Com o interruptor desligado, não se envia nem se mostra; ao
  desligar, quem me via online recebe "offline".
* App aberta: "online" a todos os contactos com sessão (não repete antes de 1 min). App
  fechada: "offline" a quem recebeu o meu "online" há menos de 5 min. Com a app aberta, de 4
  em 4 min, um batimento a quem está online.
* Um "online" vale 5 min a contar do envio (`sent_at`, limitado à hora local). Um "online"
  que ficou na fila do relay mais do que isso não conta.
* Ao receber "online" de um contacto, com a app aberta, respondo com o meu (se não lho mandei
  há menos de 30 s): quem acabou de abrir a app fica logo a saber quem está online.
* O `msg_id` é sempre o mesmo: uma versão antiga da app mostra o aviso "atualiza a app" uma só
  vez por contacto. Testes: `ffi/tests/presence.rs`.

## 17. Notificações com a app fechada (FCM)

Implementado em `server/src/push.rs` (relay) e `android/.../Push.kt` (app). Testes:
`server/tests/relay_push.rs` e `server/tests/relay_push_send.rs` (servidor falso no lugar da Google).

**Registo.** A app, sempre que se liga, envia `RegisterPushToken { user_id, token }` com o token
do Firebase Cloud Messaging deste telemóvel (só letras, dígitos e `-_:.`, até 4096). Só o próprio
`user_id` autenticado; `token` vazio apaga (interruptor "Notificações" desligado). No Redis:
`<prefixo>:push:<user_id>`, com TTL de 90 dias (como o bundle), apagado também com `DeleteAccount`.

**Campainha.** Depois de pôr uma mensagem na fila, se ninguém desse destinatário estiver em
`WaitMessages` (app fechada ou em segundo plano há mais de 30 s) e não tiver havido campainha nos
últimos 10 s, o relay envia à Google (FCM HTTP v1, conta de serviço OAuth) uma mensagem só com o
token, `priority: HIGH`, `ttl: 3600s`, `collapse_key: qt` e o campo fixo `{"qt": "1"}`. Não leva o
remetente, o conteúdo, o tamanho nem o `user_id`. Um token que a Google diga já não existir é
apagado. Sem `QT_FCM_CREDENTIALS` no relay, as notificações ficam desligadas.

**Só para mensagens.** Recibos, reações, convites, presença e envelopes de grupo que não são
mensagens (info, saída, reações) vão com `SendMessage{…, silent: true}` (`OutboxItem.silent`,
`relay_request_send_item`): o relay entrega-os, mas não toca. Antes tocava para tudo, por isso
alguém abrir a app (presença a todos os contactos) ou ler uma mensagem acordava os outros com
"Tens mensagens novas". O relay fica a saber que um envio é "silencioso", não o conteúdo nem
quem o enviou. Um relay antigo ignora o campo (toca sempre); uma app antiga não o envia.

**Chamadas (desde a app 0.9.6, aprovado a 19/10/2026).** A oferta (`OUTBOX_CALL`) e o cancelamento
antes de atender (`OUTBOX_CALL_END`) vão com `wake: "Call"` / `"CallEnd"` (`OutboxItem.wake`). O
relay toca com `{"qt": "call"}` / `{"qt": "call_end"}`, `ttl: 60s` e `collapse_key: qt-call`, com
limites próprios: uma campainha de chamada por destinatário a cada 10 s, e o "parar" só até 60 s
depois dessa campainha e uma vez. Serve para uma app com o bloqueio ativo (que não decifra nada)
tocar "Chamada SEALER" sem nome ou parar. Custo aceite: o relay e a Google ficam a saber o momento
de uma chamada (não quem liga nem nada da chamada). Um relay antigo ignora `wake` e toca como numa
mensagem.

**Na app (até à 0.9.5).** Ao receber a campainha, a app mostrava sempre a mesma notificação, "Tens
mensagens novas", sem abrir a base de dados nem decifrar nada em segundo plano.

**Na app (desde a 0.9.6).** A campainha abre a base de dados em segundo plano (a chave do SQLCipher
passou para uma chave do Keystore que funciona com o telemóvel bloqueado, depois do primeiro
desbloqueio; ver `Keystore.kt`), vai buscar o que chegou, decifra, confirma, envia os recibos e a
outbox, e trata as chamadas (`SealerCore` em `android/.../Core.kt`, um só `QtClient` por processo,
partilhado com o ecrã). A notificação das mensagens continua genérica, "Tens mensagens novas", sem
nome nem texto. Uma chamada de um contacto guardado toca com o nome, com a notificação de ecrã
inteiro do Android (por cima do bloqueio do telemóvel, com Atender e Recusar). Com o bloqueio da app
ativo nada se decifra: mensagens com o aviso genérico; `call` toca "Chamada SEALER" sem nome e o
código é pedido ao atender; `call_end` para o toque. Nos primeiros 30 s em segundo plano a app
ainda está ligada (o relay não toca) e trata tudo ela própria.

## 18. Chamadas de voz 1 a 1 (sinalização, cifra, relay, áudio e app Android)

Desenho aprovado a 19/10/2026: só Android primeiro, sem ligação direta entre telemóveis (o áudio
passa sempre pelo relay, o contacto nunca vê o IP). Fase 1 no núcleo (`core/src/call.rs`,
`ffi/src/call.rs`, testes em `ffi/tests/calls.rs`); fase 2 no relay das chamadas
(`server/src/call_relay.rs`, `server/src/bin/call_relay.rs`, testes em
`server/tests/call_relay_e2e.rs`); fase 3 no motor de áudio (`call/`, crate `qt_call`, testes em
`call/tests/engine_e2e.rs`). App Android: fase 4a (0.9.4, com a app aberta) e 4b (0.9.6, a tocar
com a app fechada ou o telemóvel bloqueado, ver secção 17).

**Sinalização (tipo 11, dentro do Double Ratchet).** Vai como as outras mensagens (envelope v3
com enchimento): o relay não sabe pelo conteúdo que é uma chamada (desde a 0.9.6 a oferta e o
cancelamento levam a campainha marcada, secção 17). `msg_id` = `call_id` (16 bytes aleatórios),
para uma versão antiga mostrar "atualiza a app" uma só vez por chamada. Corpo `[1 op][16 call_id]`:

| op | nome | resto | campainha |
|---|---|---|---|
| 1 | oferta | `[32 offer_secret][2 pad BE]` | sim, marcada `Call` (desde a 0.9.6) |
| 4 | a tocar | vazio | não (`silent`) |
| 2 | resposta | `[32 answer_secret]` | não |
| 3 | desligar | `[1 motivo]`: 1 desligou, 2 recusou, 3 ocupado, 4 sem resposta, 5 cancelou | só "cancelou" e "sem resposta" de quem liga, marcada `CallEnd` (desde a 0.9.6) |

* Uma chamada de cada vez. Uma oferta que chegue com mais de 45 s (`sent_at`, limitado à hora
  local) não toca: fica "chegou tarde" (chamada perdida). Sem resposta em 45 s, quem liga envia
  "sem resposta" e quem recebe deixa de tocar.
* Oferta com outra chamada em curso: responde "ocupado" sozinha. Os dois a ligar um ao outro ao
  mesmo tempo: fica a chamada com o `call_id` menor (os dois lados decidem igual).
* Cancelar antes de a oferta sair do telemóvel: sai da outbox e não se envia nada.
* Contactos bloqueados: a oferta é descartada como qualquer mensagem. Quem não está nos
  contactos mas tem sessão pode ligar; é a app que decide se toca.
* Tudo fica só em memória (nada em disco), como a presença.

**Chaves da chamada.** A sala só depende da oferta, para os dois lados entrarem enquanto o
telemóvel toca: `HKDF-SHA256(salt = call_id, ikm = offer_secret)` dá o `room_id` (8 bytes,
`"QT-CALL-v1 room"`) e a `relay_key` (32, `"QT-CALL-v1 relay"`). O áudio usa os dois segredos:
`HKDF-SHA256(salt = call_id, ikm = offer_secret ‖ answer_secret)` dá uma chave ChaCha20-Poly1305
por sentido (`"QT-CALL-v1 caller->callee"`, `"QT-CALL-v1 callee->caller"`). A autenticação e a
proteção pós-quântica vêm do ratchet.

**Pacote de áudio (estilo WireGuard, tamanho fixo).**
`[1 0x10 | papel][8 room_id][8 contador BE][ChaCha20-Poly1305([2 tamanho][frame][zeros até pad]) + 16][8 etiqueta]`

* Papel: 0 quem liga, 1 quem atende. Um pacote refletido de volta é recusado.
* Cabeçalho (17 bytes) como dado adicional do AEAD; nonce `[4 zeros][8 contador LE]`.
* Janela anti-repetição de 1024 pacotes (RFC 6479): aceita fora de ordem, recusa repetidos e
  muito antigos. A janela só avança depois de o AEAD passar.
* Todos os pacotes de uma chamada têm o mesmo tamanho (`pad`, 128 por agora: 169 bytes por
  pacote), para o tamanho não revelar o que se diz. Frames vazios servem para abrir a sala.
* Etiqueta = `HMAC-SHA256(relay_key, tudo o que vem antes)[..8]`: só serve para o relay aceitar
  a mudança de IP (Wi-Fi para 4G) sem que outra pessoa desvie a sala. Não decifra nada.

**Entrar na sala (salas anónimas).** `[1 0x20][Noise_N_25519_ChaChaPoly_SHA256]` cifrado para a
chave estática do relay (a do pin), com prólogo `"QT-CALL-JOIN-v1"` e conteúdo
`[1 versão=1][1 papel][8 room_id][32 relay_key]` (91 bytes no total). O relay fica a saber a sala,
o papel e a chave das etiquetas; nunca a conta. Responde `[1 0x21][8 room_id][1 o outro já está][8
etiqueta]` (18 bytes, sempre menos do que recebeu) e avisa quem já lá estava quando o outro chega.
A app repete o pedido a cada ~500 ms até à resposta e a cada 10 s enquanto não houver áudio.

**Relay das chamadas (`qt-call-relay`).** Processo à parte do relay das mensagens: recebe UDP da
Internet na porta 443, não tem acesso ao Redis nem a `/etc/qt` (só lê a chave Noise). Reserva para
redes que bloqueiam UDP: `wss://<relay>/call` (WebSocket sem conta, pelo HAProxy, para
`127.0.0.1:9445`), uma mensagem binária por pacote; os dois lados podem usar transportes diferentes.

* Encaminha o áudio de um lado para o outro, só com etiqueta válida. Não decifra nada.
* Mudança de IP (Wi-Fi para 4G): um pacote com etiqueta válida e contador maior do que o último
  desse lado, vindo de outro endereço, passa a ser o endereço desse lado. Um pacote antigo
  repetido de outro sítio não desvia a sala. Um novo pedido de entrada também muda o endereço.
* Limites: 2 lados por sala; a sala fecha se o segundo não entrar em 60 s ou ao fim de 30 s sem
  nada; 2 000 salas no total e 20 por IP (redes móveis com CGNAT); 60 pedidos de entrada por IP a
  cada 10 s; 100 pacotes/s por lado. Sem registos com IPs: uma linha por minuto só com contagens.

**Áudio (fase 3, `qt_call`).** libopus 1.6.1 (crate `opusic-sys` 0.7.5, licença BSD, compilado com
CMake), com o DRED e o PLC neural (os modelos vêm no pacote).

* Opus: 48 kHz mono, frames de 20 ms, voz em banda larga, VBR com restrição e teto de 126 bytes
  (cabe no `pad` 128 do pacote, que tem sempre o mesmo tamanho), sem DTX e sem o FEC interno
  (LBRR). Débito pedido 36 kbit/s com perda esperada de 10% e DRED até 500 ms: o libopus tira os
  bits do DRED ao débito total, e na prática cada pacote cobre ~200 ms para trás. Medido: ~90
  bytes por frame. Descodificador com complexidade 5 (PLC neural).
* Jitter buffer (`call/src/jitter.rs`): atraso alvo de 40 a 200 ms, pelo percentil 95 da variação
  das chegadas dos últimos 3 s. Frame em falta com um pacote mais à frente: perdido, reconstruído
  com o DRED desse pacote. Sem nada à frente: espera até 200 ms, contados no relógio e não em
  pedidos do altifalante (PLC sem avançar, o atraso cresce 20 ms de cada vez) e depois desiste.
  As estatísticas separam os perdidos na rede dos que não chegaram a tempo. Se depois de desistir
  voltar a chegar um pacote mais antigo do que o seguinte (o outro lado parou e retomou atrasado em
  relação ao tempo real), recomeça nele e corta o excesso até ao atraso alvo. Atraso a mais durante
  0,5 s: salta um frame.
* Motor (`call/src/engine.rs`): a app entrega 20 ms do microfone (`push_capture`) e pede 20 ms
  para tocar (`pull_playback`); a rede corre numa thread própria. Entra na sala (pedido a cada
  500 ms até à resposta, a cada 10 s depois, o que mede também a ida e volta ao relay), envia por
  UDP e passa para `wss://<relay>/call` se o UDP não responder em 2 s.
* Ferramenta do PC `tools/call-demo` (fora do workspace, precisa do ALSA): modo eco, a voz vai ao
  relay e volta, com `--perda N` para simular pacotes perdidos.

**API (`QtClient`).** `call_start`, `call_accept`, `call_hangup` (o motivo sai do estado),
`call_events` (Incoming, Ringing, Answered, Ended; chamar depois de cada lote recebido e de
segundo a segundo durante uma chamada), `call_current`, `call_join_packet` (com a chave pública
do relay), `call_join_ok`, `call_seal` e `call_open`. Com a feature `calls` (app Android):
`call_audio_start(call_id, relay_url, relay_public_key)` devolve um `CallAudio` (motor da fase 3,
já na sala, mesmo antes de atender) com `push_capture` e `pull_playback` (20 ms de PCM 16 bits,
48 kHz, mono, 1920 bytes little-endian), `stats`, `active` e `stop`; ao atender, as chaves do
áudio passam para o motor, e ao terminar a chamada ele para sozinho. As chaves nunca atravessam
a FFI.

## 19. Foto de perfil (desde a app 0.9.9)

Cada pessoa escolhe uma foto (a original até 20 MB desde a app 0.9.11; antes 3 MB). A app
corta-a ao centro e reduz para um quadrado em WebP (256 px, ou menos até caber) com no máximo
9 900 bytes, mostra-a para confirmar e, com "Aplicar", o núcleo
(`ffi/src/profile.rs`) envia-a aos contactos guardados no tipo 12, dentro do Double Ratchet.
Com o cabeçalho cabe no maior balde do enchimento (10 240), por isso o relay vê um envelope
do mesmo tamanho que uma mensagem de texto longa e não sabe que é uma foto. Nada vai ao
armazenamento de anexos do relay.

- **Tamanho:** o payload inteiro (27 bytes de cabeçalho mais a foto) tem de caber nos 10 000
  bytes de plaintext, por isso o núcleo aceita fotos até 9 973 bytes (`MAX_PHOTO_BYTES`). Até à
  0.9.10 o limite era 10 000 só para a foto: uma foto reduzida entre 9 974 e 10 000 bytes passava
  a validação mas era recusada ao cifrar (`"plaintext"`), e a app dizia "Essa imagem não serve".
- **72 horas:** só se escolhe uma foto nova 72 h depois da anterior (`profile_photo_set` devolve
  `InvalidInput { field: "photo_wait" }`). Tirar a foto pode-se sempre e não reinicia o prazo.
  Quem recebe não aceita uma foto nova do mesmo contacto antes de 71 h (hora local), por isso
  uma app alterada não troca mais depressa.
- **Só contactos:** a app passa a lista dos contactos guardados; `profile_photo_sync` manda o
  estado atual (foto ou "sem foto") a quem ainda não tem essa versão (contactos novos, sessões
  novas), uma só vez por versão. Precisa de sessão: um contacto acabado de adicionar recebe a
  foto depois da primeira troca de mensagens.
- **Guardado:** na base de dados cifrada (SQLCipher, tabela `kv`). A foto de um contacto apaga-se
  com o contacto. No ecrã, só se mostram imagens até 1024 px de lado.
- Versões antigas da app (até à 0.9.8) não conhecem o tipo 12: mostram "atualiza a app" no
  máximo uma vez por contacto (o `msg_id` é sempre o mesmo).

Testes: `ffi/tests/profile.rs`.

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
