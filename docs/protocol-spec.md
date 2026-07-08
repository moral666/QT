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

## 4. Armazenamento local — não implementado

SQLCipher para persistir: identity key (protegida por Keystore/Secure
Enclave), `RatchetState` por contacto, mensagens (se não configuradas para
desaparecer imediatamente), pre-keys não consumidas.

## 5. Sealed sender — não implementado

Camada adicional sobre as mensagens já cifradas pelo Double Ratchet, para
que o servidor de transporte não veja o remetente. A implementar após a
camada de transporte básica estar funcional.

## Próximos documentos a escrever

- `threat-model.md` — modelo de ameaças formal (quem é o adversário, o que
  ele consegue ver em cada camada)
- `architecture-decisions/0001-escolha-x3dh-vs-pqxdh.md`
- `architecture-decisions/0002-uniffi-ffi.md`
