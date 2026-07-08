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

✅ **Camada de transporte também compilada e testada de verdade**: Noise
Protocol Framework (`Noise_XX_25519_ChaChaPoly_SHA256`) sobre WebSocket real,
usando `snow` + `tokio-tungstenite`. O teste em `demo/tests/e2e_over_real_websocket.rs`
prova as duas camadas juntas: uma mensagem cifrada pelo Double Ratchet
atravessa um WebSocket real em `localhost`, protegida por um canal Noise, e
é corretamente decifrada do outro lado.

🚧 **Ainda falta**, antes de qualquer uso real:

- [ ] Sealed sender
- [ ] Bindings FFI (uniffi) para Android/iOS
- [ ] Armazenamento local (SQLCipher)
- [ ] Servidor/relay real (o teste atual simula ambos os lados no mesmo processo)
- [ ] TLS (`wss://`) - o transporte atual usa `ws://` puro, adequado só para testes locais
- [ ] Auditoria de segurança externa

**Não usar em produção ainda** — falta o servidor relay real e a app cliente.

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
