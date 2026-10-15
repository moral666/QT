# Referência da API FFI (`QtClient`, UniFFI)

Crate `qt_ffi`, módulo `ffi/src/client.rs`. Tipos: `bytes`, `string`,
`records`, `enums` e erros; nenhuma estrutura interna do Rust é exposta.
Nomes Kotlin em camelCase (`createIdentity`, `startSession`, …).
Todas as chamadas são síncronas e *thread-safe*.

## Objeto `QtClient`

| Método | Descrição |
|---|---|
| `QtClient.open(path, storageKey: 32 bytes)` | Abre/cria o storage SQLCipher com chave binária. `storageKey` é apagado da memória Rust após o uso. Erro `InvalidInput("storage_key")` se ≠ 32 bytes; `Storage` se a chave estiver errada ou o ficheiro corrompido. |
| `hasIdentity()` | Existe identidade? |
| `createIdentity() → PublicIdentity` | Gera identidade (X25519+Ed25519), signed pre-key e prekey ML-KEM-768. `IdentityExists` se já houver. |
| `exportPublicIdentity() → PublicIdentity` | Só dados públicos. |
| `deleteIdentity()` | Apaga identidade, prekeys, sessões, Noise (`secure_delete` + `VACUUM`). |
| `generatePrekeyBundle(count) → PublishedBundle` | Gera `count` (≤ 500) one-time prekeys novas com ids nunca reutilizados e devolve o bundle público. As privadas ficam no storage. |
| `publishableBundle() → bytes` | Bundle binário v2 (1349 bytes, sem OTPK) para registar no relay. |
| `startSession(publicId, PeerBundle) → contactId` | Verifica o bundle contra o QT Public ID e a assinatura; executa PQXDH (X25519 + ML-KEM-768) e cria a sessão Double Ratchet. Os dois lados partilham o mesmo segredo; a app nunca o vê. |
| `hasSession(contactId)` / `deleteSession(contactId)` | |
| `encrypt(contactId, plaintext) → envelope` | Cifra e **avança e persiste** o ratchet atomicamente. Plaintext ≤ 10 000 bytes. |
| `decrypt(contactId, envelope) → plaintext` | Rejeita replay (`Replay`) e corrupção; uma falha **não** altera o estado. Cria a sessão no 1.º envelope de um contacto novo. |
| `encryptSealed(contactId, plaintext) → blob` | Como `encrypt`, com a identidade do remetente numa caixa selada para o destinatário. |
| `decryptSealed(blob) → SealedDecrypted{contactId, plaintext}` | Abre a caixa, identifica o remetente, decifra. |
| `relayNoiseSession() → NoiseSession` | Sessão Noise (iniciador) com a chave estática Noise do dispositivo (guardada no storage). |
| `relayUserId()` | Caixa de correio no relay (`hex(signing_public)`). |
| `relayAuthMessage(handshakeHash)` | 1.ª mensagem de cada ligação ao relay (assinatura ligada ao handshake). |
| `relayRequestRegisterBundle / FetchBundle / Send / Fetch / Ack` | Constroem os pedidos ao relay (bytes a cifrar com o `NoiseSession`). |

## Funções livres

`relayParseResponse(bytes) → RelayResponse` · `peerBundleFromRelayBytes(bytes) → PeerBundle` · `relayIdOfPublicId(publicId) → string`.

## Objeto `NoiseSession`

`writeStep()`, `readStep(msg)`, `isFinished()`, `handshakeHash()`,
`remoteStaticPublicKey()`, `encrypt(pt)`, `decrypt(ct)`. Uma sessão por
ligação; nunca reutilizar após erro.

## Records / enums

`PublicIdentity{identityPublic, signingPublic, relayId, publicId}` ·
`PublishedBundle` · `PeerBundle` · `OneTimePreKeyPublic{id, public}` ·
`SealedDecrypted` · `RelayDelivered{id, sealedFrom, ciphertext}` ·
`RelayResponse{Ack | Bundle | BundleNotFound | Messages | Error}` ·
`QtError` (ver `android-integration.md` §5).

## Formatos

* **QT Public ID:** `"qt1:" + hex(identity_public[32] ‖ signing_public[32])`.
* **Bundle v2:** `[2][identity 32][signing 32][signed_pre_key 32][assinatura 64][pq_id 4 BE][ML-KEM pub 1184]`; a assinatura Ed25519 cobre `"QT-PQXDH-v1-bundle" ‖ signed_pre_key ‖ ML-KEM pub` (a v1 só cobria a signed pre-key e foi rejeitada: permitia a um relay trocar a chave ML-KEM).
* **Envelope v1 / selado / relay:** ver `protocol-spec.md` §8–§10.

## Garantias de segurança da API

A app **nunca** recebe: chaves privadas, chain/root/message keys, chaves DH do
ratchet, o segredo PQXDH, estados serializados do ratchet nem a chave estática
Noise. A API antiga (`legacy-api`) que o fazia **não** é compilada na build
Android.
