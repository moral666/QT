# Especificação do Protocolo (rascunho v0.1)

## Estado deste documento

Rascunho inicial. Cobre o que já está implementado, **compilado e testado**
em `core/`, e o que falta antes de qualquer uso real. Não é ainda uma
especificação formal revisada por pares.

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
WebSocket, implementado em `transport/` (crate `secure_messenger_transport`),
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
- **Pinning de chave estática Noise**: `NoiseHandshake::remote_static_public_key()`
  já expõe a chave do par remoto após o handshake, mas ainda não há lógica
  de comparação com um "known_hosts" persistido entre sessões.
- **Servidor/relay real**: o teste atual simula ambos os lados (cliente e
  "servidor") no mesmo processo de teste. Falta um binário de servidor
  real, com fila de mensagens e lógica de entrega assíncrona (ver secção
  seguinte do roadmap geral do projeto).

## 4. Servidor/relay — implementado e testado

Implementado em `server/` (crate `secure_messenger_server`): fila de
mensagens em memória + diretório de pre-keys públicas, comunicando através
do canal Noise/WebSocket já validado na secção anterior.

Protocolo de aplicação (`server/src/protocol.rs`, serializado em JSON por
simplicidade nesta fase):
- `RegisterPreKeyBundle` / `FetchPreKeyBundle` — diretório de pre-keys
- `SendMessage { from, to, ciphertext }` / `FetchMessages` — fila de
  entrega assíncrona. O campo `from` existe para o destinatário saber qual
  sessão/ratchet usar ao decifrar — **o servidor vê `from` e `to`
  diretamente neste momento** (não há sealed sender ainda, ver secção 6).

Testado em `server/tests/relay_flow.rs`, incluindo o caso mais importante
de um relay: **entrega assíncrona real** — Alice liga-se, envia, desliga-se;
só depois Bob se liga (ligação TCP separada) e recebe da fila, decifrando
corretamente com o Double Ratchet do lado dele.

**Pendente antes de produção:**

- **Persistência real**: a fila e o diretório de pre-keys vivem em memória
  (`server/src/store.rs`) — perdem-se se o processo reiniciar. Substituir
  por Redis (fila, com TTL automático) e uma base de dados para os bundles.
- **Sealed sender**: o campo `to`/`user_id` no protocolo atual identifica
  diretamente o destinatário (e o remetente é implícito na ligação
  autenticada) — falta a camada adicional que impede o servidor de saber
  quem enviou, só quem recebe.
- **Persistência da chave estática Noise do servidor**: gerada de novo a
  cada arranque (`server/src/bin/relay_server.rs`) — impede que clientes
  façam pinning da identidade do servidor entre reinícios.
- **Federação**: por agora um único servidor. Protocolo servidor-servidor
  para federação real fica para uma fase posterior.
- **Rate limiting / autenticação de registo**: neste momento qualquer
  ligação pode registar um bundle para qualquer `user_id` — falta lógica
  de autenticação (ex.: provar posse da identity key correspondente antes
  de aceitar um `RegisterPreKeyBundle`).

## 5. Armazenamento local (no cliente) — implementado e testado

Implementado em `storage/` (crate `secure_messenger_storage`): SQLCipher
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

## 6. Sealed sender — não implementado

Ver nota na secção 4. A implementar depois da persistência real do servidor.

## Próximos documentos a escrever

- `threat-model.md` — modelo de ameaças formal (quem é o adversário, o que
  ele consegue ver em cada camada)
- `architecture-decisions/0001-escolha-x3dh-vs-pqxdh.md`
- `architecture-decisions/0002-uniffi-ffi.md`
