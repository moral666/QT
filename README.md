# secure-messenger (nome provisório)

Mensageiro E2EE open source, focado em privacidade extrema, resistência a
backdoors e anonimato no registo. Ver `docs/protocol-spec.md` para a
especificação completa do protocolo e `apresentacao-projeto.md` (na raiz do
pacote entregue) para uma explicação em linguagem simples.

## Estado atual do projeto

🚧 **Fase 0 — fundação.** Existe apenas o núcleo criptográfico (`core/`),
como prova de conceito didática. **Não usar em produção ainda.** Faltam,
entre outras coisas:

- [ ] PQXDH (adicionar ML-KEM/Kyber ao X3DH para resistência pós-quântica)
- [ ] Assinatura Ed25519 da signed pre-key + verificação
- [ ] One-time pre-keys
- [ ] Camada de transporte (Noise sobre WebSocket)
- [ ] Sealed sender
- [ ] Bindings FFI (uniffi) para Android/iOS
- [ ] Auditoria de segurança externa

## Estrutura do repositório

```
core/           # Núcleo Rust: X3DH + Double Ratchet (esta é a parte que já existe)
  src/
    primitives.rs   # DH, HKDF, AEAD - primitivas isoladas para auditoria
    x3dh.rs         # Handshake inicial de estabelecimento de sessão
    ratchet.rs       # Double Ratchet (forward secrecy contínua)
  tests/
    full_flow.rs     # Teste de integração Alice<->Bob

docs/
  protocol-spec.md         # Especificação do protocolo (a expandir)
  threat-model.md          # Modelo de ameaças (a escrever)
  architecture-decisions/  # ADRs

android/        # (a criar) App Android nativo consumindo core/ via FFI
ios/            # (a criar) App iOS nativo consumindo core/ via FFI
desktop/        # (a criar) App Tauri consumindo core/ via FFI
server/         # (repositório separado - ver justificativa em CONTRIBUTING.md)
```

## Como compilar e testar o core

```bash
cd core
cargo test
```

Isto executa `tests/full_flow.rs`, que simula uma conversa completa
Alice↔Bob: handshake X3DH, troca de mensagens, resposta (forçando um DH
ratchet step), mensagens fora de ordem, e um teste de adulteração que deve
falhar (garante que o AEAD rejeita ciphertext modificado).

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
