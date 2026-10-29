# SEALER para iPhone (0.9.33)

**Testada a 0.9.15** (20/10/2026) com a app Android 0.9.16, sem erros: contactos, QR, partilha do
ID, menus, mensagens de texto e voz, GIFs, ficheiros, bloquear, apagar conversas e chamadas
(iPhone com iPhone e com Android). Ver `docs/field-tests.md`.

O projeto Xcode continua a chamar-se `SealerMessenger` por dentro (pastas, alvo e ficheiro
`.xcodeproj`); o nome que aparece no iPhone é **SEALER**. Identificador: **`sealer.app`**, o mesmo da
app Android.

A mesma app que a versão Android (posta a par na 0.9.13), com o **mesmo núcleo Rust** (PQXDH + ML-KEM-768, Double
Ratchet, Sealed Sender, Noise com pinning, SQLCipher, grupos, presença, fotos de perfil, chamadas com
Opus) e o **mesmo ecrã** (design 0.9.11: cabeçalho com a curva, logo a rodar só no ecrã inicial,
Definições, Novo contacto, letra Plus Jakarta Sans). Só a interface é escrita em SwiftUI. As duas apps
falam entre si pelo mesmo relay.

| Android (Kotlin)              | iPhone (Swift)                                            |
|-------------------------------|-----------------------------------------------------------|
| `MainViewModel.kt` + `Core.kt` | `Model/AppModel.swift` (um só por processo, `AppModel.shared`) |
| `MainActivity.kt`, `Design.kt` | `Views/*.swift` (`HomeScreen`, `SettingsScreen`, `ChatScreen`...) |
| `Groups.kt`                   | `Views/GroupScreens.swift`                                 |
| `Composer.kt`, `Voice.kt`, `Files.kt` | `Views/Composer.swift`, `Views/MediaSupport.swift`  |
| `Attachments.kt`              | `Views/AttachmentViews.swift`                              |
| `Calls.kt`, `CallScreen.kt`   | `Calls/CallKit.swift`, `Calls/CallAudioIO.swift`, `Views/CallScreen.swift` (`CallBar`: chamada minimizada, 0.9.29) |
| `Links.kt`                    | `Views/Links.swift`: links com confirmação do site, 0.9.29 |
| `Push.kt` (Firebase)          | `Calls/Push.swift` (Apple, **sem Firebase**)               |
| `Keystore.kt`, `AppLock.kt`   | `Security/SecureBox.swift` (Secure Enclave, Face ID)       |
| `ContactStore.kt`             | `Security/ContactStore.swift`                              |
| `Badges.kt`                   | `Views/Badges.swift` (aro do badge, `CIFalseColor`)       |
| `ReportDialog` (`MainActivity.kt`) | `ReportSheet` (`Views/ChatScreen.swift`): denunciar, 0.9.18 |
| `Recovery.kt` (`RecoveryScreen`, `RestoreScreen`) | `Views/RecoveryViews.swift`: chave de recuperação, 0.9.19 |
| `Premium.kt` (`PremiumScreen`) | `Views/PremiumViews.swift`: Sealer Premium, 0.9.20 (sem interruptores de teste desde a 0.9.21) |
| `Profile.kt`                  | `Views/MediaSupport.swift` (`ProfilePhoto`) e `Views/CallScreen.swift` (`ProfilePhotoSheet`) |
| `Lang.kt` + `res/values*/strings.xml` | `App/Lang.swift` + `Resources/*.lproj` (gerados)   |
| `RelayLink.kt`                | `Network/RelayLink.swift`                                  |
| `Qr.kt` (ZXing)               | `Views/QrViews.swift` (câmara do sistema, sem bibliotecas) |
| `BuildConfig` (gradle)        | `Configs/*.xcconfig` → `Info.plist` → `App/Config.swift`   |

## O que precisas

* Um **Mac** com **Xcode 15 ou mais recente** (o SDK do iOS só existe no macOS).
* **Rust 1.88+**, **XcodeGen** e **cmake** (o libopus das chamadas): o `compilar-iphone.sh` instala
  o que faltar (o XcodeGen e o cmake pelo Homebrew).
* Um iPhone com **iOS 17+** e o cabo.
* Conta Apple:
  * **Grátis (Apple ID normal):** chega para instalar no TEU iPhone. A app deixa de abrir ao fim de
    7 dias. **Sem notificações**: a Apple não deixa contas grátis usar "Push Notifications", por isso
    as mensagens só chegam com a app aberta e as chamadas só tocam com ela aberta.
  * **Apple Developer Program (99 €/ano):** notificações, chamadas com a app fechada, TestFlight e
    App Store.

## Passos (no Mac)

O mais simples é o zip preparado no PC (`QT-sealer-iphone.zip`): duplo clique e

```bash
cd ~/Downloads/QT-sealer && bash compilar-iphone.sh
```

Ou a partir do GitHub:

```bash
git clone https://github.com/moral666/QT.git && cd QT
git checkout main
bash compilar-iphone.sh       # Rust para iPhone + simulador, xcodegen e abre o Xcode
```

No Xcode:

1. Alvo **SealerMessenger** → **Signing & Capabilities** → **Team**: o teu Apple ID.
   Se aparecer "bundle identifier não disponível", cria `ios/Configs/Local.xcconfig` com
   `PRODUCT_BUNDLE_IDENTIFIER = pt.<teunome>.sealer` e corre o `xcodegen` outra vez.
2. Liga o iPhone, escolhe-o no topo da janela e carrega **▶ Run**.
3. Na 1.ª vez, no iPhone: **Definições → Geral → VPN e gestão de dispositivos** → confia no teu Apple ID,
   e **Definições → Privacidade e segurança → Modo de programador**.

`ios/Configs/Local.xcconfig` (não vai para o git) serve para não repetir isto sempre que corres o
`xcodegen`:

```
DEVELOPMENT_TEAM = XXXXXXXXXX
// So com a conta paga: notificacoes e chamadas com a app fechada.
QT_ENTITLEMENTS = SealerMessenger/SealerMessenger.entitlements
```

### Debug vs Release (igual ao Android)

| | Debug (▶ normal) | Release (Product → Archive) |
|---|---|---|
| Relay | editável em "Opções de teste"; por omissão o de produção (`wss://relay.sealerapp.com`) | **fixo**: `Configs/Relay.xcconfig` |
| `ws://` sem TLS | permitido | recusado (no código e no ATS) |
| Botão "Relay (teste)" | visível | escondido |

Para um relay no próprio Mac (simulador): "Opções de teste" → `ws://127.0.0.1:9443` e a chave dele.

## Notificações e chamadas (sem Firebase)

* O iPhone **não leva o SDK da Google**. O relay fala diretamente com a Apple (`server/src/apns.rs`).
* Mensagens: a Apple entrega um aviso com o texto **da própria app** (chave `push_new`, "Tens
  mensagens novas"), sem remetente nem conteúdo, como no Android.
* Chamadas: um push VoIP vazio (PushKit) acorda a app e o **CallKit** mostra a chamada, também com o
  iPhone bloqueado. Sem o bloqueio da app aparece o nome do contacto; com ele, "Chamada SEALER" e o
  código é pedido ao abrir a app (atender no ecrã do iPhone fica à espera do código).
* As chamadas **não ficam no histórico do telefone** (`includesCallsInRecents = false`), que o iOS
  sincroniza com o iCloud.
* O token enviado ao relay é `apns:<p|s>:<alertas>:<voip>` (`p` App Store/TestFlight, `s` apps
  instaladas pelo Xcode).
* No relay: `/etc/qt/apns.json` com `team_id`, `key_id`, `topic` (`sealer.app`) e a chave `.p8`
  (developer.apple.com → Keys → "Apple Push Notifications service"). O `setup-ec2.sh` liga-as se o
  ficheiro existir. Nunca no git nem na conversa.

## Segurança (o que muda em relação ao Android)

* **Chave do SQLCipher**: 32 bytes aleatórios, cifrados por uma chave P-256 **dentro do Secure
  Enclave** (não exportável), nunca copiada para backups. Como no Android desde a 0.9.6 ("1B"),
  funciona com o iPhone bloqueado depois do primeiro desbloqueio (`AfterFirstUnlockThisDeviceOnly`,
  ficheiro "QTK3"), para tocar uma chamada com a app fechada. Os ficheiros da 0.9.3 ("QTK2", só com o
  iPhone desbloqueado) passam ao formato novo sozinhos.
* **Bloqueio da app**: o código (Argon2id, no Rust) cifra a chave da base de dados ("QTL1"); sem ele
  nada abre. Face ID / Touch ID / código do iPhone guardam uma segunda cópia no Keychain, que o iOS
  invalida se mudar o rosto ou o dedo registado.
* Lista de contactos com duas camadas (Secure Enclave + AES-256-GCM com a chave derivada da do
  armazenamento): com o bloqueio ativo, também só abre com o código.
* Ficheiros em `Application Support/QT/`, **excluídos do backup**. Ficheiros abertos para ver
  (Quick Look) e envios a meio ficam na pasta temporária com a proteção máxima e são apagados ao
  fechar, ao bloquear e ao abrir a app.
* Fotos de perfil em JPEG (o iPhone não grava WebP; o Rust e o Android aceitam os dois), sem
  metadados nem localização.
* Ao sair da app, o conteúdo é tapado (lista de apps recentes).
* `PrivacyInfo.xcprivacy`: sem rastreio, sem recolha de dados.

## Idiomas

PT, EN, FR, ES, DE, IT, JA, ZH (chinês simplificado, `zh-Hans.lproj`), HI e AR (da direita para a esquerda: usar `leading`/`trailing`, nunca esquerda/direita fixas; `.offset(x:)` não se vira sozinho), **os mesmos textos da app Android**: `python3 scripts/strings-ios.py` gera
`Resources/<língua>.lproj/Localizable.strings` e `InfoPlist.strings` a partir de
`android/app/src/main/res/values*/strings.xml` (mais os poucos textos só do iPhone, no próprio
script). Correr sempre que mudar um texto no Android. A língua segue a do iPhone; a escolha manual
está em **Definições do iPhone → SEALER → Idioma** (o botão "Idioma" da app abre lá).

## Diferenças em relação ao Android

* Recibos "entregue" com a app fechada: no Android a app decifra em segundo plano; no iPhone só
  quando a abres (fazê-lo exigia uma segunda app dentro da app, a extensão de notificações, a abrir a
  mesma base de dados).
* Com o bloqueio da app ativo, se quem liga desistir, o iPhone toca até ao fim (45 s): o iOS não
  deixa um push VoIP que não mostre uma chamada nova.
* CallKit: a Apple não aceita apps com CallKit na App Store da China continental (escolher os países
  na App Store Connect).

## App Store

* Exportação: `ITSAppUsesNonExemptEncryption = YES`. Na App Store Connect: usa criptografia
  normalizada, código aberto e público, mercado de massas (ver `docs/export-compliance.md`).
* Background Modes: `audio` e `voip` (só para as chamadas).
* Termos de utilização, bloquear contactos e apagar a conta dentro da app (exigidos para apps com
  conversas entre utilizadores).
