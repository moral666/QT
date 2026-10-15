# Referência da API FFI (`QtClient`, UniFFI)

Crate `qt_ffi`: `ffi/src/client.rs` (identidade, sessões, relay),
`ffi/src/history.rs` (histórico e fila de saída), `ffi/src/attachments.rs`
(imagens e GIFs) e `ffi/src/applock.rs` (bloqueio da app). Tipos: `bytes`,
`string`, `records`, `enums` e erros; nenhuma estrutura interna do Rust é
exposta. Nomes Kotlin e Swift em camelCase (`createIdentity`, `startSession`, …).
Todas as chamadas são síncronas e *thread-safe* (correr fora da thread da interface).

## Objeto `QtClient`: identidade, sessões e relay

| Método | Descrição |
|---|---|
| `QtClient.open(path, storageKey: 32 bytes)` | Abre/cria o storage SQLCipher com chave binária. `storageKey` é apagado da memória Rust após o uso. Erro `InvalidInput("storage_key")` se ≠ 32 bytes; `Storage` se a chave estiver errada ou o ficheiro corrompido. Os anexos ficam em `<path>.anexos/`. |
| `hasIdentity()` | Existe identidade? |
| `createIdentity() → PublicIdentity` | Gera identidade (X25519+Ed25519), signed pre-key e prekey ML-KEM-768. `IdentityExists` se já houver. |
| `exportPublicIdentity() → PublicIdentity` | Só dados públicos. |
| `deleteIdentity()` | Apaga identidade, prekeys, sessões, histórico, chave Noise e a pasta dos anexos (`secure_delete` + `VACUUM`). |
| `generatePrekeyBundle(count) → PublishedBundle` | Gera `count` (≤ 500) one-time prekeys novas com ids nunca reutilizados e devolve o bundle público. As privadas ficam no storage. |
| `publishableBundle() → bytes` | Bundle binário v3 (1349 bytes, sem one-time prekeys) para registar no relay. |
| `startSession(publicId, PeerBundle) → contactId` | Verifica o bundle contra o QT Public ID e a assinatura; executa PQXDH (X25519 + ML-KEM-768) e cria a sessão Double Ratchet. Os dois lados partilham o mesmo segredo; a app nunca o vê. |
| `hasSession(contactId)` / `deleteSession(contactId)` | |
| `safetyNumber(peerPublicId) → string` | 60 dígitos para comparar com o contacto por outro canal ("Verificar segurança"). |
| `encrypt(contactId, plaintext) → envelope` | Cifra e **avança e persiste** o ratchet atomicamente. Plaintext ≤ 10 000 bytes. |
| `decrypt(contactId, envelope) → plaintext` | Rejeita replay (`Replay`) e corrupção; uma falha **não** altera o estado. Cria a sessão no 1.º envelope de um contacto novo. |
| `encryptSealed(contactId, plaintext) → blob` | Como `encrypt`, com a identidade do remetente numa caixa selada para o destinatário. |
| `decryptSealed(blob) → SealedDecrypted{contactId, plaintext}` | Abre a caixa, identifica o remetente, decifra. |
| `relayNoiseSession() → NoiseSession` | Sessão Noise (iniciador) com a chave estática Noise do dispositivo (guardada no storage). |
| `relayUserId()` | Caixa de correio no relay (`hex(signing_public)`). |
| `relayAuthMessage(handshakeHash)` | 1.ª mensagem de cada ligação ao relay (assinatura ligada ao handshake). |
| `relayRequestRegisterBundle / FetchBundle / Send / Fetch / Ack` | Constroem os pedidos ao relay (bytes a cifrar com o `NoiseSession`). Confirmar (`Ack`) só depois de processar cada mensagem. |

As apps usam `encrypt*`/`decrypt*` só através das funções de histórico abaixo,
que fazem a cifra e a gravação na mesma transação.

## Histórico e fila de saída

| Método | Descrição |
|---|---|
| `sendText(contactId, text) → OutboxItem` | Numa só transação: avança o ratchet, guarda a mensagem como "por enviar" e põe o envelope selado na fila de saída. A app envia `item.blob` ao relay e, quando ele confirmar, chama `outboxSent(item.outboxId)`. |
| `outbox() → [OutboxItem]` | Tudo o que o relay ainda não confirmou, pela ordem de criação (reenviar depois de falhas ou reinícios). |
| `outboxSent(outboxId)` | O relay confirmou: sai da fila e a mensagem passa a "Enviada". |
| `receiveSealed(blob) → ReceiveResult` | Decifra, guarda e agenda o recibo "entregue", numa transação. Só confirmar ao relay depois de devolver Ok (ou um erro definitivo como `Replay`). |
| `prepareDeliveryReceipts() → u32` | Junta os recibos "entregue" pendentes (um envelope por contacto) na fila de saída. Chamar depois de processar um lote. |
| `markConversationRead(contactId, sendReadReceipt) → u32` | Marca as recebidas como lidas e, se `sendReadReceipt`, põe o recibo "vista" na fila (é o interruptor "Notificações" das apps). |
| `messagesPage(contactId, beforeSeq?, limit)` | Página do histórico, da mais recente para a mais antiga. |
| `messagesAfter(contactId, afterSeq)` | Mensagens mais recentes que `afterSeq`. |
| `message(contactId, messageId) → ChatMessage?` | Estado atual de uma mensagem. |
| `conversations() → [ConversationSummary]` | Lista de conversas (última mensagem + por ler), a mais recente primeiro. |
| `deleteConversation(contactId)` | Apaga as mensagens, o que estava por enviar e os anexos desta conversa; a sessão mantém-se. |
| `deleteContact(contactId)` | Apaga também a sessão cifrada e os metadados do contacto; os registos anti-replay passam a hashes anónimos. Se o contacto voltar a ser adicionado, começa uma sessão nova. |

## Imagens e GIFs (anexos)

Formato e fluxo em `protocol-spec.md`, secção 14. A app usa uma segunda
ligação ao relay só para os ficheiros.

| Método | Descrição |
|---|---|
| `attachmentCreate(contactId, sourcePath, mime, width, height) → ChatMessage` | Cifra o ficheiro temporário (já comprimido pela app; JPEG, PNG, GIF ou WebP, até 25 MiB) com uma chave nova. A mensagem fica no histórico como "a enviar". |
| `attachmentsPendingUpload()` / `attachmentsPendingDownload() → [AttachmentRef]` | Anexos por enviar / por descarregar (retomar depois de falhas). |
| `attachmentBlobId(contactId, messageId) → string?` | `None` = ainda sem espaço reservado no relay. |
| `blobCreateRequest(…)` → depois `attachmentSetBlob(…, blobId, token)` | Reservar espaço no relay e guardar o id e o token devolvidos (`BlobCreated`). |
| `blobInfoRequest(…)` | Quantos bytes o relay já tem (para retomar). |
| `blobPutRequest(…, offset)` | O bloco cifrado que começa em `offset` (múltiplo de 40 KiB). |
| `attachmentUploaded(…) → OutboxItem` | O relay tem o ficheiro inteiro: envia a mensagem com a chave (cifrada pelo Double Ratchet) pela fila de saída. |
| `blobGetRequest(…, offset)` / `attachmentWriteChunk(…, offset, data) → u64` | Descarregar um bloco (`BlobData`) e guardá-lo; devolve os bytes já no telemóvel. |
| `attachmentDownloadedBytes(…) → u64` | Bytes já descarregados (para retomar). |
| `attachmentDownloadFinished(…) → ChatMessage` | Verifica o ficheiro inteiro (todos os blocos, a ordem e o fim): fica "pronto" ou "falhou" (e é apagado). |
| `blobDeleteRequest(…)` | Pedir ao relay que apague o ficheiro, depois de descarregado e verificado. |
| `attachmentFailed(…, expired) → ChatMessage` | O envio ou a descarga falhou de vez (`expired`: o relay já não tem o ficheiro). |
| `attachmentRead(contactId, messageId) → bytes` | A imagem em claro, só para mostrar (em memória, nunca escrita em disco). |

## Funções livres

`relayParseResponse(bytes) → RelayResponse` · `peerBundleFromRelayBytes(bytes) → PeerBundle` · `relayIdOfPublicId(publicId) → string`.

Bloqueio da app: `appLockWrap(code, storageKey) → bytes` (cifra a chave do
storage com o código, Argon2id, sal e nonce novos em cada chamada) ·
`appLockUnwrap(code, blob) → storageKey` (código errado ou dados alterados:
`Crypto`) · `appLockMinCodeLen()`.

## Objeto `NoiseSession`

`writeStep()`, `readStep(msg)`, `isFinished()`, `handshakeHash()`,
`remoteStaticPublicKey()`, `encrypt(pt)`, `decrypt(ct)`. Uma sessão por
ligação; nunca reutilizar após erro.

## Records / enums

`PublicIdentity{identityPublic, signingPublic, relayId, publicId}` ·
`PublishedBundle` · `PeerBundle` · `OneTimePreKeyPublic{id, public}` ·
`SealedDecrypted` · `RelayDelivered{id, sealedFrom, ciphertext}` ·
`RelayResponse{Ack | Bundle | BundleNotFound | Messages | Error | BlobCreated | BlobStatus | BlobData | BlobNotFound}` ·
`ChatMessage{seq, messageId, contactId, outgoing, text, sentAtMs, localAtMs, status, attachment?}` ·
`MessageStatus{Pending | Sent | Delivered | Read | Received | ReadByMe}` ·
`ConversationSummary{contactId, lastMessage, unread}` ·
`OutboxItem{outboxId, contactId, blob}` ·
`ReceiveResult{NewMessage | StatusChanged | NoChange}` ·
`AttachmentInfo{mime, size, width, height, state, transferSize}` ·
`AttachmentState{Uploading | Downloading | Ready | Failed | Expired}` ·
`AttachmentRef{contactId, messageId}` ·
`QtError` (ver `android-integration.md` §5).

## Formatos

* **QT Public ID:** `"qt1:" + hex(identity_public[32] ‖ signing_public[32])`.
* **Bundle v3:** `[3][identity 32][signing 32][signed_pre_key 32][assinatura 64][pq_id 4 BE][ML-KEM pub 1184]`; a assinatura Ed25519 cobre a identidade X25519, a signed pre-key e a chave ML-KEM (a v1 só cobria a signed pre-key e permitia a um relay trocar a chave ML-KEM; a v2 não ligava a identidade X25519). Detalhe em `protocol-spec.md` §9.
* **Envelope v2 / selado / relay:** ver `protocol-spec.md` §10 e §11.
* **Anexos:** ver `protocol-spec.md` §14.

## Garantias de segurança da API

A app **nunca** recebe: chaves privadas, chain/root/message keys, chaves DH do
ratchet, o segredo PQXDH, estados serializados do ratchet, a chave estática
Noise nem as chaves dos anexos. A API antiga (`legacy-api`) que expunha chaves
**não** é compilada nas builds Android e iOS.
