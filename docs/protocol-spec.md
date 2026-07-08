# Especificação do Protocolo (rascunho v0.1)

## Estado deste documento

Rascunho inicial. Cobre o que já está implementado em `core/` e o que falta
antes de qualquer uso real. Não é ainda uma especificação formal revisada
por pares — isso é um requisito antes da Fase 1 (MVP) começar, conforme
`README.md` do repositório principal do projeto (ver conversa de arquitetura
anterior, seção "Roadmap").

## 1. Handshake inicial (X3DH) — implementado, incompleto

Implementado em `core/src/x3dh.rs`: 3x Diffie-Hellman (X25519) combinados
via HKDF-SHA256, seguindo a estrutura do X3DH original (Marlinspike/Perrin,
Signal, 2016).

**Pendente antes de produção:**

- **PQXDH**: adicionar um KEM pós-quântico (ML-KEM/Kyber, FIPS 203) ao lado
  do X25519 no cálculo do segredo compartilhado, para resistência a ataques
  "colher agora, decifrar depois" por um adversário com computador quântico
  futuro. O Signal já fez isso em produção — usar a mesma estrutura como
  referência.
- **Assinatura da signed pre-key**: a `signed_pre_key` no `PreKeyBundle`
  deve ser assinada com a `identity_key` (Ed25519) no momento da publicação,
  e essa assinatura **verificada** por quem inicia o handshake, antes de
  prosseguir. Sem isso, um servidor malicioso pode substituir a chave
  publicada (ataque de personificação). Marcado como TODO em `x3dh.rs`.
- **One-time pre-keys**: adicionar uma quarta DH (`EK_A x OPK_B`) usando uma
  chave descartável de uso único, consumida do servidor a cada novo
  handshake. Fortalece a garantia de forward secrecy da própria primeira
  mensagem.

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

## 3. Camada de transporte — não implementado

Decisão de arquitetura (da conversa de design): Noise Protocol Framework
sobre WebSocket, usando a crate `snow`. Isto fica numa camada separada do
`core/`, que deve permanecer agnóstica de rede — `core/` só produz/consome
bytes cifrados, nunca faz I/O de rede diretamente (facilita testes e
auditoria).

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
