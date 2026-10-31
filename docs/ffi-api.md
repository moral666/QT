# Referência da API FFI (`qt_ffi`, UniFFI)

Índice da API que as apps Android e iPhone usam (0.9.69). O crate `qt_ffi` expõe só `bytes`,
`string`, records, enums e erros; nenhuma estrutura interna do Rust passa para as apps. Em Kotlin
e Swift os nomes ficam em camelCase (`create_identity` → `createIdentity`). As chamadas são
síncronas e *thread-safe*: as apps correm-nas fora da thread da interface.

Quase tudo é um método do objeto `QtClient`, dividido por ficheiros em `ffi/src/`. Os formatos
estão em [`protocol-spec.md`](protocol-spec.md) (a secção indicada em cada parte).

**Features do crate:** `pq` (por omissão: `QtClient` com PQXDH), `calls` (chamadas de voz e de
grupo, motor `qt_call`), `video` (videochamadas) e `legacy-api` (API antiga com chaves em bytes,
só para testes; nunca nas builds das apps). As apps compilam com `pq,calls,video`.

## `client.rs`: identidade, sessões e relay (§8 a §11)

| Método | O que faz |
|---|---|
| `QtClient.open(path, storageKey)` | Abre ou cria o SQLCipher com a chave binária de 32 bytes (apagada da memória do Rust depois do uso). Os anexos ficam em `<path>.anexos/`. |
| `hasIdentity()` / `createIdentity()` / `exportPublicIdentity()` / `deleteIdentity()` | Identidade X25519 + Ed25519, signed pre-key e prekey ML-KEM-768. Apagar leva tudo: chaves, sessões, histórico e anexos. |
| `generatePrekeyBundle(count)` / `publishableBundle()` | One-time prekeys novas (≤ 500) e o bundle v4 (1361 bytes) para o relay. |
| `startSession(publicId, PeerBundle)` / `hasSession` / `deleteSession` | Verifica o bundle contra o QT Public ID e a assinatura e faz o PQXDH. |
| `safetyNumber(peerPublicId)` | 60 dígitos para comparar fora de banda. |
| `encrypt` / `decrypt` / `encryptSealed` / `decryptSealed` | Cifra e decifra com o Double Ratchet (com o remetente selado nas `*Sealed`). As apps usam-nas só através do histórico. |
| `relayNoiseSession()` / `relayUserId()` / `relayAuthMessage(hash)` | Sessão Noise com a chave estática do telemóvel, caixa no relay e a 1.ª mensagem de cada ligação (assinatura ligada ao handshake). |
| `relayRequestRegisterBundle` / `FetchBundle` / `Send` / `SendItem` / `Fetch` / `Wait` / `Ack` | Pedidos ao relay (bytes a cifrar com o `NoiseSession`). `Wait` espera até 25 s por mensagens. Confirmar (`Ack`) só depois de guardar cada mensagem. |
| `relayRequestDeleteAccount()` / `relayRequestPushToken(token)` | Apagar a conta no relay; token das notificações com a app fechada. |

Funções livres: `relayParseResponse(bytes)`, `peerBundleFromRelayBytes(bytes)`,
`relayIdOfPublicId(publicId)`, `relayRecipientDeletedReason()`, `relayElsewhereName(reason)` e
`relayNoNewAccountsReason()` (federação: conta noutro relay, relay sem contas novas).

## `relay_choice.rs`: relay de uma conta nova (desde a 0.9.69, §25.2)

| Função | O que faz |
|---|---|
| `relayOrderForNewAccount(relays, timeoutMs)` | Mede só a ligação TCP a cada relay da lista (sem TLS nem pedidos) e devolve os nomes do mais rápido para o mais lento; os que não respondem ficam no fim. Bloqueia até `timeoutMs` (no máximo 10 s). |
| `relayNextForNewAccount(order, tried)` | O primeiro dessa ordem que ainda não respondeu "sem contas novas". |

## `history.rs`: histórico, fila de saída e contactos (§12, §13)

| Método | O que faz |
|---|---|
| `sendText(contactId, text)` / `react(contactId, messageId, emoji)` | Numa transação: avança o ratchet, guarda e põe o envelope selado na fila de saída. |
| `outbox()` / `outboxSent(id)` / `outboxDrop(id)` | O que o relay ainda não confirmou; confirmado; retirado sem enviar (o destinatário apagou a conta). |
| `receiveSealed(blob)` | Decifra, guarda e agenda o recibo "entregue". Só confirmar ao relay depois de devolver Ok. |
| `prepareDeliveryReceipts()` / `markConversationRead(contactId, sendReadReceipt)` | Recibos "entregue" e "vista". |
| `messagesPage` / `messagesAfter` / `message` / `conversations()` | Leitura do histórico e da lista de conversas. |
| `deleteConversation` / `deleteContact` / `deleteContactNotify` | Apagar mensagens, ou também a sessão (com o aviso "sessão terminada" ao outro lado). |
| `sessionsToRestart()` | Contactos que terminaram a sessão e com quem é preciso abrir uma nova. |
| `blockContact` / `unblockContact` / `isBlocked` / `blockedContacts()` | Bloquear: as mensagens desse contacto são descartadas ao chegar. |
| `pendingIntro` / `pendingIntros()` / `dismissIntro` | Convites de quem escreveu sem estar nos contactos. |

## `attachments.rs`: fotos, GIFs, voz e ficheiros (§14)

`attachmentCreate` (imagens e GIFs) e `attachmentCreateFile` (qualquer ficheiro até 25 MiB)
cifram o ficheiro com uma chave nova. Envio: `blobCreateRequest` → `attachmentSetBlob` →
`blobInfoRequest` / `blobPutRequest` (blocos de 40 KiB, retoma) → `attachmentUploaded` (a
mensagem com a chave vai pela fila de saída). Receção: `blobGetRequest` →
`attachmentWriteChunk` → `attachmentDownloadFinished` (verifica tudo) → `blobDeleteRequest`.
Também `attachmentsPendingUpload` / `PendingDownload`, `attachmentBlobId`,
`attachmentDownloadedBytes`, `attachmentFailed` e `attachmentRead` (em memória, nunca em disco).

## `groups.rs`: grupos (até 10 pessoas, §15)

`groupCreate`, `groups()`, `group`, `groupUpdate` (só administradores), `groupLeave`,
`groupDelete`, `groupMembersNeedingSession` (abrir sessões antes de enviar), `groupSendText`,
`groupAttachmentUploaded`, `groupReact`, `groupMessagesPage` e `groupIdOfConversation`.

## `call.rs` e `call_audio.rs`: chamadas a dois (§18)

| Método | O que faz |
|---|---|
| `callStart` / `callStartVideo` / `callAccept` / `callHangup` | Ligar (voz ou vídeo), atender, desligar, recusar ou cancelar. A sinalização vai pela fila de saída. |
| `callEvents()` | Eventos desde a última chamada (toca, atendida, terminou, pedido de vídeo…). Chamar depois de cada lote de `receiveSealed` e de segundo a segundo durante a chamada. |
| `callCurrent()` / `callIsVideo` / `callVideoSupported` / `callVideoContacts()` | Estado da chamada e a quem se pode fazer videochamada. |
| `callVideoSetCamera(on)` / `callVideoRespond(resposta)` | Ligar ou desligar a câmara e responder ao pedido de vídeo do outro lado. |
| `callContactsSync(contactIds)` | A lista de contactos da app (decide a ligação direta entre contactos mútuos) e o anúncio "tenho vídeo". |
| `callSetRelay` / `callRelay()` | Federação: o relay da conta e o relay das chamadas desta chamada. |
| `callJoinPacket` / `callJoinOk` / `callSeal` / `callOpen` | Sala no relay das chamadas e cifra dos pacotes de áudio (todos do mesmo tamanho). |

`callAudioStart(…) → CallAudio` arranca o motor (Opus 1.6 com DRED, jitter buffer). O objeto
`CallAudio`: `pushCapture` / `pullPlayback` (20 ms, `callFrameBytes()`), `videoParams`,
`videoPush`, `videoPull`, `videoRequestKeyframe`, `videoHold`, `videoSetEncodeLimit`,
`videoSetDecodeLimit`, `setNetwork`, `stats`, `active`, `stop` e `diagnostic` (números para os
testers, sem IPs nem IDs).

## `group_call.rs`: chamadas de grupo (até 5 pessoas, desde a 0.9.62, §27)

| Método | O que faz |
|---|---|
| `groupCallNoticeAccepted()` / `groupCallAcceptNotice()` | O aviso das chamadas de grupo, mostrado uma vez antes da primeira. Sem "Aceitar" não se liga nem se entra. |
| `groupCallStart(groupId, video)` / `groupCallJoin` / `groupCallLeave` | Ligar a um grupo, entrar na chamada que está a tocar, sair ou recusar. |
| `groupCallOngoing(groupId)` / `groupCallRejoin` | A chamada a decorrer num grupo em que não estou, e entrar depois (0.9.65). |
| `groupCallSetName(callId, name)` | O meu nome nesta chamada, até 32 caracteres (0.9.67). |
| `groupCallSetCamera(on)` | Ligar ou desligar a minha câmara. |
| `groupCallEvents()` | Eventos desde a última vez. Chamar depois de cada lote de `receiveSealed` e de segundo a segundo durante a chamada. |
| `groupCallCurrent()` / `groupCallGroup` / `groupCallIsVideo` / `groupCallParticipants` | Estado da chamada e quem está nela. |

`groupCallAudioStart(…) → GroupAudio` arranca o áudio e o vídeo da chamada. O objeto
`GroupAudio`: `pushCapture` / `pullPlayback` (a mistura de todos), `speaker()` (quem fala, para
o quadrado grande), `meSpeaking()`, `videoParams`, `videoPush`, `videoPull(contactId)`,
`videoRequestKeyframe`, `videoHold`, `videoSetEncodeLimit`, `videoSetDecodeLimit`, `setNetwork` e
`diagnostic` / `diagnosticOf` (números para os testers, sem IPs nem IDs).

## Outros módulos

| Ficheiro | Métodos | Secção |
|---|---|---|
| `presence.rs` | `presenceConfigure`, `presenceAnnounce`, `presenceHeartbeat`, `presenceOnline`; livre: `presenceHeartbeatMs()` | §16 |
| `profile.rs` | `profilePhotoSet`, `profilePhotoRemove`, `profilePhotoSync`, `profilePhotoMine`, `profilePhotoNextChangeMs`, `profilePhotoOf` | §19 |
| `badges.rs` | `badgeSync`, `badgeMine`, `badgeOf`, `badgeDeliver` (só para a ferramenta das medalhas) | §20 |
| `report.rs` | `relayRequestReport`; livre: `reportsAvailable()` | §21 |
| `recovery.rs` | `recoveryEnabled`, `recoveryEnable`, `recoveryWords`, `recoveryDisable`, `recoveryBackup`, `recoveryBackupSent`, `recoveryPrepare`, `recoveryRestore`, `relayRequestBackupGet`, `relayRequestBackupDelete`; livre: `recoveryIsWord(word)` | §22 |
| `community.rs` | `communitySubmitRequest`, `communitySubmitSent`, `relayRequestCommunityVersion` / `Index` / `Photos`, `communityCachedVersion`, `communityApplyIndex`, `communityMissingPhotos`, `communityPutPhoto`, `communityMembers` | §26 |
| `applock.rs` | Livres: `appLockWrap(code, storageKey)` (Argon2id, sal e nonce novos), `appLockUnwrap(code, blob)`, `appLockMinCodeLen()` | |

## Objeto `NoiseSession`

`writeStep()`, `readStep(msg)`, `isFinished()`, `handshakeHash()`, `remoteStaticPublicKey()`,
`encrypt(pt)`, `decrypt(ct)`. Uma sessão por ligação; nunca reutilizar depois de um erro.

## Erros

`QtError` (ver [`android-integration.md`](android-integration.md) §5). Os erros de entrada
dizem o campo (`InvalidInput { field }`), por exemplo `storage_key`, `palavras`,
`group_call_full` ou `call_busy`.

## Garantias de segurança da API

A app **nunca** recebe: chaves privadas, chain, root ou message keys, chaves DH do ratchet, o
segredo PQXDH, estados serializados do ratchet, a chave estática Noise, as chaves dos anexos nem as
chaves das chamadas. A API antiga (`legacy-api`), que expunha chaves, **não** é compilada nas
builds Android e iPhone.
