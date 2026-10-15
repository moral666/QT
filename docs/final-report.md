# Relatório final: fase "core seguro + FFI para Android"

> **Registo de uma fase anterior (13 de outubro de 2026).** Ficou
> como estava na altura: por exemplo, o histórico de mensagens passou depois
> para o SQLCipher do Rust, o bundle passou à v3 e há hoje mais de 160 testes.
> O estado atual está no `README.md` e em `known-limitations.md`.

> **Este trabalho NÃO está auditado e NÃO deve ser descrito como "seguro" ou
> "production-ready".** Os testes passam; isso demonstra que o código faz o
> que os testes verificam, não que o protocolo ou a implementação sejam
> seguros contra adversários reais.

## IMPLEMENTADO
- **PQXDH** (X25519 + ML-KEM-768) sem `expect`/panic, com erros tipados;
  **serialização/persistência** das chaves ML-KEM com a API real do `ml-kem 0.2.3`
  (`EncodedSizeUser`, 2400/1184/1088 bytes) e validação canónica.
- **Bundle PQXDH assinado incluindo a chave ML-KEM** (v2): corrige a possibilidade de um relay anular a proteção PQ.
- **`QtClient`** (UniFFI): identidade (criar/exportar/apagar), QT Public ID, bundles,
  `startSession` (verifica bundle + PQXDH), `encrypt/decrypt`, `encryptSealed/decryptSealed`,
  sessão Noise com chave guardada no Rust, mensagens de autenticação/pedidos ao relay e codec.
  Nenhuma chave privada ou estado de ratchet atravessa a FFI (API antiga atrás de `legacy-api`).
- **Storage SQLCipher:** chave binária de 32 bytes (ponte para o Keystore), `secure_delete`
  verificado por leitura, tabelas para prekeys de uso único, prekeys PQ, chave Noise,
  metadados de sessão, deduplicação de envelopes; sem tabela de mensagens; `wipe_all`.
- **Relay endurecido:** autenticação Ed25519 com *channel binding*, isolamento por utilizador,
  entrega *at-least-once* com `AckMessages`, limites, timeouts, tratamento de Ping, sessão
  encerrada em frames inválidos; **`RelayClient`** com reconnect, backoff, timeouts, pinning.
- **Double Ratchet:** `decrypt` transacional (mensagem forjada não quebra a sessão).
- **Auditoria interna** (secrets, RNG, `unwrap`/`expect`, logs, serialização) e correções.
- **Build Android:** feature `android` (SQLCipher+OpenSSL embebidos), `scripts/build-android.sh`,
  `generate_bindings.sh` com verificação de que o Kotlin não expõe a API legacy.
- **Documentação:** `threat-model.md`, `protocol-spec.md` §8–11, `android-integration.md`,
  `ffi-api.md`, `known-limitations.md`.

## TESTADO (88 testes Rust + 2 scripts Python via UniFFI; 0 falhas)
- PQXDH Alice→Bob com segredos iguais; dependência dos componentes clássico e PQ; chave pública/privada/ciphertext inválidos; assinatura inválida; substituição da chave ML-KEM; persistência com "reinício" (12 testes + fluxo).
- Double Ratchet: ida e volta, restauro de estado, replay, corrupção, fora de ordem, estado forjado, estado serializado corrompido (fuzz de truncagem/bit-flip) e **forward secrecy / post-compromise** (11 testes).
- Storage: chave errada, ficheiro corrompido/truncado, ficheiro sem cabeçalho SQLite, `secure_delete`, prekey reutilizada, dedup, `wipe_all` (14 + 3).
- API `QtClient` diretamente (13): conversa nos dois sentidos, reinício, replay, adulteração, prekey reutilizada/forjada, bundles inválidos, Sealed Sender (relay sem plaintext nem identidade), ciclo de vida da identidade.
- Relay (16 + 2): sem autenticação, assinatura reutilizada, acesso cruzado, at-least-once, limites, fila cheia, frames Noise corrompidos/repetidos/fora de ordem, Ping, timeouts, reconnect, relay em baixo e recuperado, pinning.
- **Dois clientes + relay + Redis reais** (3): conversa com quedas e reinícios de clientes e relay, duplicados, Redis sem plaintext, e o caminho Android (WebSocket + `NoiseSession` + codec).

## NÃO IMPLEMENTADO
- Compilação/execução real dos `.so` Android (script escrito, **não executado**) e a app Kotlin.
- One-time prekeys *single-use* servidas pelo relay; rotação de prekeys; validade de bundles.
- Anexos além de imagens e GIFs (estes já existem, até 25 MiB: ver `protocol-spec.md`, 14); *session reset*; *safety numbers*; backup.
- Rate limiting/anti-spam; federação; TLS configurado de fábrica.
- Correção do `pn` do ratchet (exige mudar o formato); *transcript binding* no HKDF.
- Fuzzing contínuo, `cargo audit`, análise de canais laterais, auditoria externa.

## LIMITAÇÕES
Ver `docs/known-limitations.md` (28 itens) e `docs/threat-model.md` §4.

## RISCOS
1. **Criptografia não auditada** (inclui `ml-kem 0.2.3`): risco principal.
2. `build-android.sh` por validar; o cross-compile de OpenSSL/SQLCipher pode exigir ajustes.
3. A app pode anular as garantias guardando histórico em claro ou mantendo a chave do storage em memória/logs.
4. Metadados expostos ao relay; relay malicioso pode censurar e atrasar.
5. Rollback do ficheiro SQLCipher não é detetado.
6. O protocolo muda (bundle v2, relay autenticado): não é compatível com v0.1.

## PRÓXIMO PASSO PARA ANDROID
1. Correr `./scripts/build-android.sh` (NDK r26+) e validar as 3 ABIs num emulador x86_64 e num aparelho arm64.
2. Implementar `StorageKeyProvider` (Keystore) e `RelayLink` (OkHttp + `NoiseSession`) seguindo `docs/android-integration.md` §2 e §4.
3. Embutir a chave pública Noise do relay (pinning) e usar `wss://`.
4. Escrever testes instrumentados que repliquem `two_clients_over_relay.rs`.
5. Encomendar auditoria criptográfica externa antes de qualquer utilização real.

## Comandos exatos
```bash
# testes (Redis local necessário)
redis-server --daemonize yes
cargo test --workspace

# bindings (Kotlin/Swift de produção + Python de teste)
./ffi/generate_bindings.sh

# .so para as 3 ABIs + bindings Kotlin
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/<versão>
./scripts/build-android.sh app/src/main
# equivalente manual:
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -P 24 -o app/src/main/jniLibs \
  build --release -p qt_ffi --no-default-features --features android
```
