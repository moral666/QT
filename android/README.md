# SEALER para Android (Kotlin / Jetpack Compose)

**Versão de testes (0.9.31).** A 0.9.16 foi testada a 20/10/2026 com a app iPhone 0.9.15, sem erros (ver `docs/field-tests.md`); a 0.9.22 foi testada a 22/10/2026 (bloqueio da app, apps recentes e recuperação da conta).
Toda a criptografia está no Rust (`libqt_ffi.so`); o Kotlin só move bytes e desenha ecrãs.
No telemóvel a app chama-se **SEALER**. O identificador na Play Store é `sealer.app` (desde a 0.9.0; antes `pt.qt.app`); o código Kotlin continua no pacote `pt.qt.app`.

```
Compose UI -> MainViewModel -> QtClient (UniFFI/Rust: identidade, PQXDH, ratchet, sealed sender, SQLCipher)
                  |
                  +-> RelayLink (OkHttp WebSocket <-> NoiseSession do Rust) -> relay_server -> Redis
                  +-> 2.ª RelayLink só para imagens (blocos de 40 KB)  -> relay_server -> disco do relay
```

| Ficheiro | O que faz |
|---|---|
| `MainActivity.kt` | Ecrãs (lista de conversas, chat, diálogos) |
| `MainViewModel.kt` | Estado da app, envio e receção, transferência de imagens |
| `RelayLink.kt` | Ligação WebSocket + Noise ao relay |
| `Attachments.kt` | Imagens e GIFs: preparar, mostrar, enviar |
| `Qr.kt` | Código QR do ID (gerar e ler) |
| `AppLock.kt` | Bloqueio da app (código próprio e biometria) |
| `Keystore.kt` | Chave do SQLCipher protegida pelo Android Keystore |
| `ContactStore.kt` | Lista de contactos cifrada |
| `Theme.kt` | Cores, logotipo e barra de topo |
| `Design.kt` | Cabeçalhos com o fundo curvo, ecrã inicial e Definições |
| `Groups.kt` | Grupos (criar, membros, administradores) |
| `Files.kt` | Ficheiros até 25 MB (abrir noutra app a partir da memória, guardar) |
| `Voice.kt` | Mensagens de voz (até 10 s) |
| `Calls.kt`, `CallScreen.kt` | Chamadas de voz (serviço do microfone, notificação de ecrã inteiro) |
| `Core.kt`, `Push.kt` | Núcleo em segundo plano e notificações (FCM) |
| `Profile.kt`, `Badges.kt` | Foto de perfil e medalhas |
| `Recovery.kt` | Chave de recuperação (12 palavras) |

## O que tens de ter

1. Os `.so` e os bindings (já os geraste no projeto Rust):
   `QT/android-out/jniLibs/{arm64-v8a,armeabi-v7a}/libqt_ffi.so` e `QT/ffi/bindings/kotlin/uniffi/...`
2. **Android Studio** (inclui JDK e SDK). Sem `snap`, no Pop!_OS:
   descarrega o `.tar.gz` em https://developer.android.com/studio e depois
   `sudo tar -xzf ~/Transferências/android-studio-*.tar.gz -C /opt && /opt/android-studio/bin/studio.sh`
3. Um **telemóvel Android** (Android 7 ou mais recente) com *Opções de programador* + *Depuração USB*.
   A APK só leva as bibliotecas `arm64-v8a` e `armeabi-v7a` (quase todos os telemóveis). O emulador do
   Android Studio também correu esta APK no teste de campo.
4. O **NDK 27 ou mais recente** (*SDK Manager* → *SDK Tools* → *NDK*). Com o r26d a compilação do
   OpenSSL incluído falhou.

## Passos

```bash
cd QT
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/<versao 27 ou mais recente>
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
As imagens enviadas ficam na pasta `anexos/` ao lado da chave Noise (aqui `$HOME/anexos`), ou em
`RELAY_BLOB_DIR` se a definires; o relay apaga-as ao fim de 24 horas.
Nota: o relay escuta por omissão só em `127.0.0.1` e recusa outro endereço sem `QT_ALLOW_PUBLIC_BIND=1`
(usar só em redes de teste). **No emulador** não é preciso: o PC é `10.0.2.2`, por isso basta o relay em
`127.0.0.1` e na app `ws://10.0.2.2:9443` (é o valor por omissão da build debug quando `qtRelayUrl` não está definido).
Em produção: `wss://` atrás de proxy TLS (`docs/deploy-wss.md`, `deploy/aws/setup-ec2.sh`).

## Testar com UM só telemóvel: o PC faz de segundo utilizador

```bash
cd ~/QT && cargo run -p qt_server --example peer -- --relay ws://127.0.0.1:9443 --pin <chave hex> --db ~/peer.db
```
1. O `peer` mostra o **QT Public ID** dele. Na app: botão do logotipo (*Novo contacto*) e cola-o.
2. Na app, em ⋮ → *O meu ID*, copia o **teu** Public ID; no `peer` escreve `/add qt1:...`
3. Escreve mensagens nos dois lados. Recebidas no `peer` aparecem como `<< texto`.

Este fluxo (dois processos + relay verdadeiro + pinning) já foi testado de ponta a ponta no PC.
O `peer` só troca texto: uma imagem enviada pela app aparece lá só como `📷 Foto` ou `GIF`.

## Definições

Toca na tua foto (com a roda dentada), no cabeçalho do ecrã inicial:

- **Foto de perfil** e **O meu ID** (em texto e em código QR).
- **Chave de recuperação**: 12 palavras que recuperam a conta num telemóvel novo.
- **Bloqueio da app** (código, biometria, tempo até bloquear), **Bloquear agora** e **Contactos bloqueados**.
- **Privacidade**: mostrar quando estou online e confirmações de leitura (o visto ✓✓).
- **Notificações**, **Idioma** e **Versão**.
- **Relay (teste)**: só na build *debug*.
- **Destruir perfil e dados**: apaga a conta no servidor e depois tudo deste telemóvel.

No chat, o cadeado abre **Verificar segurança** e o menu ⋮ tem **Bloquear**, **Denunciar** e **Eliminar conversa ou contacto**.

## Adicionar alguém que está ao teu lado (código QR)

Em *O meu ID*, a app mostra o ID também como código QR. No outro telemóvel: *Novo contacto*,
*Ler código QR* e apontar a câmara. O ID fica preenchido; falta só o nome. O código é gerado e
lido no telemóvel (bibliotecas ZXing e ZXing Android Embedded, Apache 2.0), sem rede nem serviços
da Google. A permissão da câmara só é pedida quando se carrega em *Ler código QR*. Na app
iPhone funciona igual (câmara do sistema, sem bibliotecas extra).

## Imagens e GIFs

No chat, o botão **+** abre a galeria; GIFs e autocolantes do teclado (Gboard, SwiftKey...)
entram diretamente no campo da mensagem. Antes de enviar há uma pré-visualização com a opção
**Qualidade original** (por omissão a foto é reduzida para 1600 px, ~300 KB). Máximo 25 MB.

A imagem é cifrada no telemóvel (chave nova por ficheiro) e segue para o relay em blocos de
40 KB, por uma segunda ligação, para não atrasar as mensagens. O relay apaga-a quando o outro
telemóvel a descarrega, ou ao fim de 24 horas. No telemóvel, os ficheiros ficam cifrados e a
imagem em claro só existe em memória enquanto está no ecrã. Detalhes em
`docs/protocol-spec.md`, secção 14; limitações em `docs/known-limitations.md`, 36.

O relay tem de ser atualizado para aceitar imagens; com um relay antigo a app avisa
"O servidor ainda não aceita imagens". O relay da AWS já foi atualizado.

## Segurança já tratada na app
Chave do SQLCipher gerada aleatoriamente e protegida pelo Android Keystore (StrongBox se existir);
`allowBackup=false`; conversas escondidas na lista de apps recentes (Android 13+ com `setRecentsScreenshotEnabled`; Android 7 a 12 com FLAG_SECURE só fora do primeiro plano, desde a 0.9.22; as capturas de ecrã são permitidas); sem logs de conteúdo/chaves; TLS obrigatório na build *release*;
bloqueio da app opcional (código próprio que cifra a chave do SQLCipher e a lista de contactos, com Argon2id no Rust, mais desbloqueio com impressão digital, rosto ou PIN do telemóvel através de uma chave do Keystore que só funciona depois de autenticar; ao bloquear, as imagens em memória são esquecidas; depois de 5 códigos errados, espera crescente contada pelo relógio desde que o telemóvel ligou); pinning da chave do relay (obrigatório na *release*); confirmação (`Ack`) só depois de processar; deduplicação feita pelo Rust.

## Limitações (ver `docs/known-limitations.md`, itens 30 a 36)
Com a app fechada chega só um aviso genérico ("Tens mensagens novas"), sem nome nem texto. Desde a 0.9.6 a app decifra em segundo plano
para as chamadas tocarem (com o bloqueio da app ativo nada é decifrado sem o código). As ligações ao relay fecham 30 s depois de a app sair
do ecrã. Anexos: o relay vê o tamanho aproximado e quem envia e descarrega cada ficheiro, e um anexo não descarregado em 24 horas perde-se.

## O que foi verificado
Compilada no Android Studio em *debug* e *release* (R8 ativo), instalada num emulador e em telemóveis reais,
e testada com mensagens trocadas entre dois telemóveis através do relay na AWS (`wss://` + pinning).
Fotos, GIFs, voz, ficheiros, grupos, chamadas de voz e notificações testados entre três Samsung
(dois Galaxy S23 e um S26) através do relay na AWS.
Ainda não há testes automáticos de interface nem testes em muitos modelos e versões de Android.

## Assinatura de release (keystore própria)

A build *release* **falha** se não houver keystore (não usa a chave de debug). Gera uma **uma vez**, guarda-a
fora do repositório e faz cópia de segurança (se a perderes, não consegues publicar atualizações):

```bash
keytool -genkeypair -v -keystore ~/qt-release.jks -alias qt -keyalg RSA -keysize 4096 -validity 10000
```
Cria `android/keystore.properties` (ignorado pelo git) ou usa as variáveis `QT_KEYSTORE_FILE`,
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

## Verificar segurança (safety number)
Na conversa, o cadeado (*Verificar segurança*) mostra 60 dígitos calculados no Rust a partir dos Public
IDs dos dois. Compara-os por outro canal; se diferirem, não confies na conversa.
