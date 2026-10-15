# SEALER para iPhone (alpha)

O projeto Xcode continua a chamar-se `SealerMessenger` por dentro (pastas, alvo e ficheiro
`.xcodeproj`); o nome que aparece no iPhone é **SEALER**.

A mesma app que a versão Android: o **mesmo núcleo Rust** (PQXDH + ML-KEM-768, Double Ratchet,
Sealed Sender, Noise com pinning, SQLCipher, histórico, recibos ✓/✓✓) e o **mesmo ecrã**
(lista de conversas estilo WhatsApp, chat com bolhas, tema roxo com o logotipo a rodar, "O meu ID"
com código QR, "Verificar segurança", imagens e GIFs). Só a interface é reescrita em SwiftUI. As duas
apps falam entre si pelo mesmo relay.

| Android (Kotlin)            | iPhone (Swift)                         |
|-----------------------------|----------------------------------------|
| `MainViewModel.kt`          | `Model/AppModel.swift`                 |
| `MainActivity.kt`           | `Views/*.swift`                        |
| `RelayLink.kt` (OkHttp)     | `Network/RelayLink.swift` (URLSession) |
| `Attachments.kt`            | `Views/AttachmentViews.swift`          |
| `Qr.kt` (ZXing)             | `Views/QrViews.swift` (câmara do sistema, sem bibliotecas extra) |
| `Keystore.kt` (Keystore)    | `Security/SecureBox.swift` (Secure Enclave) |
| `ContactStore.kt`           | `Security/ContactStore.swift`          |
| `AppLock.kt`                | ainda não existe (ver Limitações)      |
| `Theme.kt`                  | `App/Theme.swift`                      |
| `BuildConfig` (gradle)      | `Configs/*.xcconfig` → `Info.plist` → `App/Config.swift` |

## O que precisas

* Um **Mac** com **Xcode 15 ou mais recente** (o SDK do iOS só existe no macOS).
* **Rust**: `curl https://sh.rustup.rs -sSf | sh`
* **XcodeGen**: `brew install xcodegen` (gera o projeto Xcode a partir de `project.yml`).
* Um iPhone com **iOS 17+** e o cabo.
* Conta Apple:
  * **Grátis (Apple ID normal):** chega para instalar no TEU iPhone. A app deixa de abrir ao fim de
    7 dias (voltas a carregar no ▶ no Xcode). Não dá para enviar a amigos.
  * **Apple Developer Program (99 €/ano):** TestFlight (até 10 000 testadores por link) e App Store.

## Passos (no Mac)

```bash
git clone https://github.com/moral666/QT.git && cd QT
git checkout security-hardening

./scripts/build-ios.sh          # compila o Rust para iPhone + simulador (demora na 1.ª vez)
cd ios && xcodegen              # cria SealerMessenger.xcodeproj
open SealerMessenger.xcodeproj
```

No Xcode:

1. Seleciona o alvo **SealerMessenger** → separador **Signing & Capabilities** → **Team**: o teu Apple ID
   (Xcode → Settings → Accounts → "+" se ainda não estiver lá).
   Se aparecer "bundle identifier não disponível", muda para algo teu, ex. `pt.nuno.sealermessenger`.
   (Para não teres de repetir isto sempre que corres o `xcodegen`, cria `ios/Configs/Local.xcconfig`
   com `DEVELOPMENT_TEAM = XXXXXXXXXX` e, se precisares, `PRODUCT_BUNDLE_IDENTIFIER = ...`.)
2. Liga o iPhone, escolhe-o no topo da janela e carrega **▶ Run**.
3. Na 1.ª vez, no iPhone: **Definições → Geral → VPN e gestão de dispositivos** → confia no teu Apple ID.
   No iOS 16+ também tens de ligar **Definições → Privacidade e segurança → Modo de programador**.

### Debug vs Release (igual ao Android)

| | Debug (▶ normal) | Release (Product → Archive) |
|---|---|---|
| Relay | editável em "Opções de teste"; por omissão `ws://127.0.0.1:9443` (relay no Mac, para o simulador) | **fixo**: `Configs/Relay.xcconfig` (o mesmo AWS da app Android) |
| `ws://` sem TLS | permitido | recusado (no código e no ATS) |
| Botão "Relay (teste)" | visível | escondido |

Para testar no iPhone com o relay da AWS em Debug: "Opções de teste" → URL
`wss://13-51-46-213.sslip.io` e a chave do relay. Ou usa o esquema **Release**
(Product → Scheme → Edit Scheme → Run → Build Configuration → Release).

O build Release falha de propósito se `QT_RELAY_URL` não for `wss://` ou se `QT_RELAY_PIN`
não tiver 64 hex.

## Imagens, GIFs e código QR

* No chat, o botão **+** abre um menu: **Galeria** (escolher fotos, sem pedir acesso a todas as
  fotos) ou **Colar imagem ou GIF**. Os GIFs do teclado (por exemplo o do Gboard) entram copiando-os
  primeiro no teclado e depois escolhendo *Colar imagem ou GIF*.
* Antes de enviar há uma pré-visualização com a opção **Qualidade original**. Máximo 25 MB. O resto
  (cifra, blocos de 40 KB, 24 horas no relay) é igual ao Android: ver `android/README.md`.
* **O meu ID** mostra também o código QR; em **Novo contacto** (o botão com o logotipo) há
  *Ler código QR*. A câmara só é pedida nesse momento (`NSCameraUsageDescription` no `Info.plist`).
* O interruptor **Notificações** no menu decide se a app avisa o outro lado de que leste as
  mensagens (o visto azul). Ainda não há notificações push.

## Segurança (o que muda em relação ao Android)

* **Chave do SQLCipher**: 32 bytes aleatórios, cifrados por uma chave P-256 **dentro do Secure
  Enclave** (não exportável), só utilizável com o iPhone desbloqueado e nunca copiada para backups
  (`WhenUnlockedThisDeviceOnly`). Nunca é criada uma chave nova por cima de uma base de dados
  existente (seria perda de dados silenciosa).
* Ficheiros em `Application Support/QT/` com **Data Protection "Complete"** e **excluídos do backup**.
* Lista de contactos cifrada pelo Secure Enclave (os metadados "quem fala com quem").
* Ao sair da app, o conteúdo é tapado (o iOS tira uma fotografia do ecrã para a lista de apps
  recentes). As capturas de ecrã são permitidas, tal como na app Android.
* `PrivacyInfo.xcprivacy`: sem rastreio, sem recolha de dados.

## Limitações desta versão

* **Compilada e testada até ao teste de campo** de 15 de outubro de 2026 (commit `d3a48ef`, no iPhone
  e no simulador do Mac). O que veio depois foi escrito sem acesso a um Mac e **ainda não foi
  compilado no Xcode**: apagar conversa e contacto, a marca SEALER e o tema, o código QR, as imagens
  e os GIFs, e o botão com o logotipo. O código Swift foi verificado contra os bindings gerados a
  partir do Rust real (nomes e tipos). Se o build der erros, copia-os e eu corrijo.
* **Sem bloqueio da app** (código próprio e Face ID), que já existe no Android.
* Como no Android, as mensagens só chegam com a app aberta (verificação a cada 3 s).
  Notificações push no iPhone exigem a conta paga (APNs) e um serviço no relay; está na lista.
* Exportação: a app usa criptografia. Ao enviar para TestFlight/App Store a Apple pergunta pela
  conformidade de exportação (responde que usa encriptação; ver a documentação da Apple).
