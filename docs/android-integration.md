# Integração Android (Kotlin / Jetpack Compose)

**Regra fundamental: nenhuma criptografia em Kotlin.** A app só move bytes,
mostra UI e gere o ciclo de vida. Todas as chaves, o ratchet, o PQXDH, o
Sealed Sender, o storage cifrado e a chave estática Noise vivem no Rust.

```
Compose UI ─► ViewModel ─► QtClient (UniFFI, Rust)   ◄── storage SQLCipher (ficheiro) + anexos cifrados
                  │
                  ├─► RelayLink (OkHttp WebSocket ⇄ NoiseSession do Rust) ─► Relay
                  └─► 2.ª RelayLink, só para imagens e GIFs               ─► Relay
```

A app de referência está em `android/` (ver `android/README.md`); a app
iPhone em `ios/` segue o mesmo desenho em Swift.

## 1. Construir `libqt_ffi.so` (3 ABIs) e os bindings

```text
Rust → cargo build → cargo-ndk → libqt_ffi.so → UniFFI → qt_ffi.kt → app
```

Pré-requisitos no PC de build: Rust ≥ 1.81 (`rustup`), Android NDK r27+ (com o r26d a compilação do OpenSSL falhou),
`perl` e `make` (compilam SQLCipher + OpenSSL a partir do código-fonte),
`cargo install cargo-ndk --locked`.

```bash
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/<versão>
rustup target add aarch64-linux-android armv7-linux-androideabi

# tudo de uma vez (as 2 ABIs da app + bindings Kotlin):
ABIS="arm64-v8a armeabi-v7a" ./scripts/build-android.sh app/src/main

# ou à mão:
cargo ndk -t arm64-v8a -t armeabi-v7a -P 24 \
  -o app/src/main/jniLibs \
  build --release -p qt_ffi --no-default-features --features android
./ffi/generate_bindings.sh           # gera ffi/bindings/kotlin/uniffi/qt_ffi/qt_ffi.kt
cp -r ffi/bindings/kotlin/uniffi app/src/main/java/
```

* `--features android` = `pq` (PQXDH) + `calls` + `video` + SQLCipher/OpenSSL embebidos. **Sem**
  `legacy-api`. Sem a feature embebida o `.so` ligaria a uma `libsqlcipher`
  que não existe no Android.
* Resultado: `jniLibs/{arm64-v8a,armeabi-v7a}/libqt_ffi.so`.
* O `generate_bindings.sh` **falha** se o Kotlin gerado expuser alguma função
  que faça chaves privadas ou estados de ratchet atravessarem a FFI.

`app/build.gradle.kts`:

```kotlin
dependencies {
    implementation("net.java.dev.jna:jna:5.17.0@aar")   // exigido pelo UniFFI Kotlin
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
}
android { defaultConfig { ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a") } } }
```

(A app só leva as duas ABIs ARM. Para um emulador x86_64, compila também essa ABI
(`-t x86_64`, com o target `x86_64-linux-android` e um NDK r27 ou mais recente) e acrescenta-a
ao `abiFilters`.)

`proguard-rules.pro`:

```text
-keep class com.sun.jna.** { *; }
-keep class uniffi.qt_ffi.** { *; }
```

`AndroidManifest.xml`: `android:allowBackup="false"` (o ficheiro cifrado não
é restaurável noutro dispositivo: a chave está no Keystore desse aparelho).

## 2. Android Keystore → chave do SQLCipher

```
Android Keystore (AES-256-GCM, não exportável)
      │ cifra
      ▼
segredo aleatório de 32 bytes (SecureRandom, gerado 1 vez)  ← guardado cifrado em ficheiro privado
      │ decifrado em cada arranque
      ▼
QtClient.open(path, key)  → Rust usa-o como chave binária do SQLCipher e APAGA a sua cópia
```

```kotlin
class StorageKeyProvider(private val ctx: Context) {
    private val alias = "qt_storage_wrap_key"
    private val file get() = File(ctx.noBackupFilesDir, "qt_storage_key.bin")

    private fun wrapKey(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(alias, null) as? SecretKey)?.let { return it }
        val spec = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setIsStrongBoxBacked(false)          // true se o aparelho tiver StrongBox (tratar StrongBoxUnavailableException)
            .setUnlockedDeviceRequired(true)      // API 28+
            .build()
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
            .apply { init(spec) }.generateKey()
    }

    /** Devolve 32 bytes. O CHAMADOR deve fazer `fill(0)` depois de abrir o QtClient. */
    fun storageKey(): ByteArray {
        if (!file.exists()) {
            val raw = ByteArray(32).also { SecureRandom().nextBytes(it) }
            val c = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, wrapKey()) }
            file.writeBytes(c.iv + c.doFinal(raw))     // IV(12) || ct||tag
            return raw
        }
        val blob = file.readBytes()
        val c = Cipher.getInstance("AES/GCM/NoPadding")
            .apply { init(Cipher.DECRYPT_MODE, wrapKey(), GCMParameterSpec(128, blob.copyOfRange(0, 12))) }
        return c.doFinal(blob, 12, blob.size - 12)
    }
}
```

Regras: a chave nunca vai para logs, `Intent`s, `SharedPreferences` em claro,
exceções nem para o Git. Depois de `QtClient.open(...)` faz `key.fill(0)`.
A chave passa brevemente pela memória da JVM (limitação inevitável: o Rust
não pode usar uma chave não exportável do Keystore diretamente). Perder a
chave do Keystore (reset de fábrica, nova biometria se exigires autenticação)
torna o ficheiro irrecuperável. É o comportamento esperado.

## 3. Ciclo de vida da identidade e dos contactos

```kotlin
val qt = QtClient.open(File(filesDir, "qt.db").path, key).also { key.fill(0) }

// 1.º arranque
if (!qt.hasIdentity()) {
    val me = qt.createIdentity()           // PublicIdentity
    showToUser(me.publicId)                // "qt1:…": o que o utilizador partilha (QR/texto)
    relay.registerBundle(qt.publishableBundle())   // publicar no relay (ver §4)
}

// Adicionar contacto: o utilizador cola/lê o QT Public ID do outro.
suspend fun addContact(publicId: String): String {
    val relayId = relayIdOfPublicId(publicId)            // caixa do outro no relay
    val bytes = relay.fetchBundle(relayId) ?: error("sem bundle")
    val bundle = peerBundleFromRelayBytes(bytes)
    return qt.startSession(publicId, bundle)             // contactId; verifica o bundle contra o Public ID
}

// Enviar: cifra, guarda no histórico e põe na fila de saída, numa transação
val item = qt.sendText(contactId, text)
relay.send(relayIdOfPublicId(publicId), item.blob)           // se falhar: fica em qt.outbox() e reenvia-se o MESMO blob
qt.outboxSent(item.outboxId)                                 // só depois do Ack do relay

// Receber (ver §4)
when (val r = qt.receiveSealed(delivered.ciphertext)) {      // decifra e guarda, numa transação
    is ReceiveResult.NewMessage -> mostrar(r.message)
    is ReceiveResult.StatusChanged -> atualizarVistos(r)
    is ReceiveResult.NoChange -> {}
}
```

* `contactId` = hex da chave pública de identidade do contacto.
* `receiveSealed` cria a sessão sozinho quando recebe a 1.ª mensagem de um
  contacto novo (o cabeçalho PQXDH vai dentro do envelope).
* Apagar conta: `qt.deleteIdentity()` (apaga tudo, `secure_delete` + `VACUUM`)
  e apagar o ficheiro + a chave do Keystore.
* **A app não deve guardar plaintext fora de storage cifrado.** O histórico
  fica no SQLCipher do Rust (`messagesPage`, `conversations`) e as imagens
  ficam cifradas no disco; `attachmentRead` devolve a imagem em claro só
  para mostrar. A app não deve escrever nem uma nem outra noutro lado.
* API completa (histórico, recibos, anexos, bloqueio): `docs/ffi-api.md`.

## 4. Ligação ao relay (WebSocket + Noise): máquina de estados

O Kotlin **não** implementa Noise: usa `NoiseSession` do Rust. A lógica de
referência (testada) está em `server/src/client.rs` (`RelayClient`) e
`server/tests/two_clients_over_relay.rs::caminho_android_*`; na app está
replicada em `RelayLink.kt` e `MainViewModel.kt`.

```
Disconnected ─► Connecting ─► NoiseHandshake ─► Authenticating ─► Ready
      ▲             │               │                 │             │
      └─────────────┴───── qualquer erro/timeout ─────┴─────────────┘  (backoff, NOVA sessão)
```

1. **Ligar:** `OkHttp` `wss://…`, `connectTimeout` ≈ 10 s.
2. **Handshake:** `val noise = qt.relayNoiseSession()`; enquanto `!noise.isFinished()`:
   `ws.send(noise.writeStep())`; se ainda não terminou, esperar frame binário e `noise.readStep(frame)`.
3. **Pinning (recomendado):** comparar `noise.remoteStaticPublicKey()` com a
   chave pública do relay embebida na app; se diferir → **abortar sem retry**
   (possível MITM). A chave pública é impressa no arranque do `relay_server`.
4. **Autenticar:** `ws.send(noise.encrypt(qt.relayAuthMessage(noise.handshakeHash())))`;
   esperar `RelayResponse.Ack` (senão `Error` → não repetir).
5. **Pedidos** (um de cada vez, resposta sequencial): construir com
   `qt.relayRequest*`, `ws.send(noise.encrypt(req))`, esperar frame com
   **timeout** (≈ 15 s), `relayParseResponse(noise.decrypt(frame))`.
   Ignorar frames Ping/Pong. Exceção: na ligação dos anexos a app envia até 8
   blocos seguidos antes de ler as respostas (o relay responde pela ordem).
6. **Qualquer erro** (timeout, falha de `decrypt`, frame inesperado, fecho):
   fechar o WebSocket, **descartar a `NoiseSession`** (nunca reutilizar) e
   voltar a *Connecting* com backoff exponencial + jitter (200 ms … 30 s).
   Um pedido pendente repete-se na nova ligação (é idempotente: o destinatário
   deduplica).
7. **Receber:** `relayRequestFetch()` → `Messages(list)`; para cada mensagem
   `receiveSealed(...)`; tratar `QtException.Replay` como já processada;
   **só depois** de persistir/mostrar tudo: `relayRequestAck(ids)`. Repetir
   fetch→ack até vir lista vazia (o relay devolve páginas).
   Mensagens permanentemente inválidas (`InvalidEnvelope`/`Crypto`) também
   se confirmam (senão ficam em fila e são re-entregues indefinidamente).
8. **Encerrar:** enviar Close (`ws.close(1000, null)`) e descartar a sessão.
   O estado criptográfico persiste no storage, nunca na ligação. Enquanto o
   Android não a fecha, a app fica à espera de mensagens (`relayRequestWait`,
   ver abaixo); com a app fechada acorda pelas notificações push.

   **Uma só ligação (desde a app Android 0.9.45 / iPhone 0.9.32):** os pedidos
   vão em fila pela mesma ligação, sem esperar pela resposta do anterior, e o
   relay responde sempre pela mesma ordem (cada resposta é do pedido mais antigo
   por responder). Antes havia três ligações: principal, anexos e espera.

   **Entrega imediata:** a app envia `relayRequestWait(25)` com tempo de
   resposta de 35 s. O relay responde no instante em que chega uma mensagem (ou
   vazio ao fim de 25 s, ou logo que chega outro pedido nessa ligação, que trata
   a seguir); as mensagens recebidas tratam-se como as de `relayRequestFetch`
   (guardar e só depois `relayRequestAck`) e volta-se a esperar. Uma espera
   vazia que acaba em menos de 1 s foi interrompida: volta-se a esperar sem
   novo `relayRequestFetch`.
   Se o relay responder `Error` com `"mensagem invalida"` (relay antigo), a
   app volta a `relayRequestFetch` de 3 em 3 s. Sem rede, também.
9. **Rede perdida:** tratar como erro (passo 6). Ao reconectar nada se perde:
   mensagens por enviar ficam na fila de saída do Rust (`outbox()`, blobs já
   cifrados) e as por receber ficam na fila do relay até ao `Ack`.
10. **Chamadas bloqueantes:** todas as chamadas a `QtClient` são síncronas
    (SQLCipher + cripto): correr em `Dispatchers.IO`. São *thread-safe*
    (serializadas por um mutex).

## 5. Erros (`QtException`)

| Variante | Significado | Ação sugerida na app |
|---|---|---|
| `InvalidInput(field)` | Argumento inválido | Bug da app / input do utilizador |
| `Storage` | Storage inacessível (chave errada, ficheiro corrompido, disco) | Mostrar erro; não tentar “reparar” apagando |
| `IdentityMissing` / `IdentityExists` | Estado de identidade | Fluxo de onboarding |
| `UnknownSession` | Sem sessão para esse contacto | Iniciar com `startSession` |
| `Replay` | Envelope já processado | Ignorar e confirmar |
| `InvalidEnvelope` | Formato inválido/corrompido | Descartar e confirmar |
| `InvalidBundle` | Bundle não corresponde ao Public ID / assinatura inválida | **Avisar o utilizador** (relay malicioso ou Public ID errado) |
| `PrekeyUnavailable` | Prekey inexistente/já usada | Pedir ao remetente para reiniciar a sessão |
| `Crypto` | Falha criptográfica (ex.: mensagem adulterada) | Descartar; não revela detalhes |

As mensagens de erro nunca contêm chaves, plaintext nem caminhos sensíveis.

## 6. Testar sem Android

```bash
cargo test --workspace                 # precisa de Redis: redis-server --daemonize yes
./ffi/generate_bindings.sh             # gera bindings e corre os testes Python via UniFFI
```

`ffi/tests/client_flow.rs` e `server/tests/two_clients_over_relay.rs`
exercitam exatamente a API que o Android usa (incluindo o caminho WebSocket +
`NoiseSession` + codec).
