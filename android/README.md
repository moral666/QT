# QT — app Android de referência (Kotlin / Jetpack Compose)

**Protótipo NÃO auditado. Não uses para conversas reais.**
Toda a criptografia está no Rust (`libqt_ffi.so`); o Kotlin só move bytes e desenha ecrãs.

```
Compose UI -> MainViewModel -> QtClient (UniFFI/Rust: identidade, PQXDH, ratchet, sealed sender, SQLCipher)
                  |
                  +-> RelayLink (OkHttp WebSocket <-> NoiseSession do Rust) -> relay_server -> Redis
```

## O que tens de ter

1. Os `.so` e os bindings (já os geraste no projeto Rust):
   `QT/android-out/jniLibs/{arm64-v8a,armeabi-v7a}/libqt_ffi.so` e `QT/ffi/bindings/kotlin/uniffi/...`
2. **Android Studio** (inclui JDK e SDK). Sem `snap`, no Pop!_OS:
   descarrega o `.tar.gz` em https://developer.android.com/studio e depois
   `sudo tar -xzf ~/Transferências/android-studio-*.tar.gz -C /opt && /opt/android-studio/bin/studio.sh`
3. Um **telemóvel Android arm64** (quase todos) com *Opções de programador* + *Depuração USB*.
   (Emulador x86_64 ainda não: falta esse `.so`.)

## Passos

```bash
cd QT
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/<versao>
ABIS="arm64-v8a armeabi-v7a" ./scripts/build-android.sh   # .so alinhados a 16 KB + bindings
cd android
./sync-native.sh ..   # copia os .so e os bindings Kotlin para app/src/main/
```
Abre a pasta `android/` no Android Studio (*Open*), espera pelo *Gradle Sync* (descarrega dependências) e carrega em ▶ com o telemóvel ligado.

## O relay no teu PC (para o telemóvel o alcançar na mesma Wi-Fi)

```bash
sudo systemctl start redis-server
cd ~/QT && cargo build --release -p qt_server --bins
QT_ALLOW_PUBLIC_BIND=1 RELAY_BIND_ADDR=0.0.0.0:9443 NOISE_KEY_PATH=$HOME/relay_noise_key.bin ./target/release/relay_server
#   -> imprime "Chave publica estatica Noise ...: <64 hex>"   (guarda-a: serve de pinning)
hostname -I          # o IP do PC, ex.: 192.168.1.23
sudo ufw allow 9443/tcp   # se tiveres firewall
```
Na app: URL `ws://192.168.1.23:9443` (o `ws://` só funciona na build *debug*) e a chave pública de pinning.
Nota: o relay escuta por omissão só em `127.0.0.1` e recusa outro endereço sem `QT_ALLOW_PUBLIC_BIND=1`
(usar só em redes de teste). **No emulador** não é preciso: o PC é `10.0.2.2`, por isso basta o relay em
`127.0.0.1` e na app `ws://10.0.2.2:9443` (é o valor por omissão da build debug quando `qtRelayUrl` não está definido).
Em produção: `wss://` atrás de proxy TLS (`docs/deploy-wss.md`, `deploy/aws/setup-ec2.sh`).

## Testar com UM só telemóvel: o PC faz de segundo utilizador

```bash
cd ~/QT && cargo run -p qt_server --example peer -- --relay ws://127.0.0.1:9443 --pin <chave hex> --db ~/peer.db
```
1. O `peer` mostra o **QT Public ID** dele. Na app: *Adicionar contacto* e cola-o.
2. Na app copia o **teu** Public ID; no `peer` escreve `/add qt1:...`
3. Escreve mensagens nos dois lados. Recebidas no `peer` aparecem como `<< texto`.

Este fluxo (dois processos + relay verdadeiro + pinning) já foi testado de ponta a ponta no PC.

## Segurança já tratada na app
Chave do SQLCipher gerada aleatoriamente e protegida pelo Android Keystore (StrongBox se existir);
`allowBackup=false`; `FLAG_SECURE` (sem capturas de ecrã); sem logs de conteúdo/chaves; TLS obrigatório na build *release*;
pinning opcional da chave do relay; confirmação (`Ack`) só depois de processar; deduplicação feita pelo Rust.

## Limitações (ver `QT/docs/known-limitations.md`, itens 29–34)
Sessões cruzadas se ambos enviarem a 1.ª mensagem em simultâneo; histórico só em memória; poll de 3 s;
sem notificações; sem x86_64 (emulador x86 só com imagens ARM); **sem auditoria**.

## O que foi verificado
Compilada no Android Studio em *debug* e *release* (R8 ativo), instalada num emulador e em telemóveis reais,
e testada com mensagens trocadas entre dois telemóveis através do relay na AWS (`wss://` + pinning).
**Não** foram feitos testes automáticos de interface, testes em muitos modelos/versões de Android, nem auditoria.

## Assinatura de release (keystore própria)

A build *release* **falha** se não houver keystore (não usa a chave de debug). Gera uma **uma vez**, guarda-a
fora do repositório e faz cópia de segurança (se a perderes, não consegues publicar atualizações):

```bash
keytool -genkeypair -v -keystore ~/qt-release.jks -alias qt -keyalg RSA -keysize 4096 -validity 10000
```
Cria `android/keystore.properties` (ignorado pelo git) — ou usa as variáveis `QT_KEYSTORE_FILE`,
`QT_KEYSTORE_PASSWORD`, `QT_KEY_ALIAS`, `QT_KEY_PASSWORD`:
```
storeFile=/home/tu/qt-release.jks
storePassword=...
keyAlias=qt
keyPassword=...
```
A release precisa também do relay de produção em `android/gradle.properties` (não são segredos):
```
qtRelayUrl=wss://<o-teu-relay>
qtRelayPin=<chave publica Noise do relay, 64 hex>
```
Nessa build o relay fica fixo: os campos de URL/chave e o botão "Relay" não aparecem.

Depois: no Android Studio, *Build Variants* → `release` → *Build → Build APK(s)*
(ou `./gradlew assembleRelease`, se tiveres o wrapper). O APK fica em `app/build/outputs/apk/release/`.
A release só aceita `wss://` (`usesCleartextTraffic=false`, `network_security_config` sem cleartext e
verificação em `RelayLink`/`saveRelay`). `./gradlew :app:verifyReleaseSignature` confirma as assinaturas V2/V3.

## Safety numbers
Na conversa, *Safety number* mostra 60 dígitos calculados no Rust a partir dos Public IDs dos dois.
Compara-os por outro canal; se diferirem, não confies na conversa.
