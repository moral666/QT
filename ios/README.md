# SEALER para iPhone (0.9.3)

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
* **Rust 1.88 ou mais recente**: `curl https://sh.rustup.rs -sSf | sh` (ou `rustup update`, se já tiveres).
* **XcodeGen**: `brew install xcodegen` (gera o projeto Xcode a partir de `project.yml`).
* Um iPhone com **iOS 17+** e o cabo.
* Conta Apple:
  * **Grátis (Apple ID normal):** chega para instalar no TEU iPhone. A app deixa de abrir ao fim de
    7 dias (voltas a carregar no ▶ no Xcode). Não dá para enviar a amigos.
  * **Apple Developer Program (99 €/ano):** TestFlight (até 10 000 testadores por link) e App Store.

## Passos (no Mac)

O código Swift da 0.9.3 (grupos, voz, ficheiros e notificações push) foi feito no Mac. Se ainda não
estiver no GitHub, envia-o primeiro (ver "Enviar o código do Mac para o GitHub", mais abaixo), senão a
app compilada a partir do GitHub fica mais antiga do que a que está no iPhone.


```bash
git clone https://github.com/moral666/QT.git && cd QT
git checkout site-sealer

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
`wss://relay.sealer.pt` e a chave do relay. Ou usa o esquema **Release**
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
  mensagens (o visto azul). As notificações push (avisos com a app fechada) já funcionam.

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

* **Versão 0.9.3 compilada no Xcode e testada num iPhone** (19 de outubro de 2026): mensagens, grupos,
  voz, fotos, ficheiros e notificações push com a app fechada.
* **Sem bloqueio da app** (código próprio e Face ID), que já existe no Android.
* Exportação: a app usa criptografia. Ao enviar para TestFlight/App Store a Apple pergunta pela
  conformidade de exportação (responde que usa encriptação; ver a documentação da Apple).

## Enviar o código do Mac para o GitHub

Na pasta do QT no Mac. O `GoogleService-Info.plist` (Firebase) e as chaves da Apple (`.p8`, `.p12`,
perfis) ficam fora do git pelo `.gitignore`: confirma na lista do `git status` que não aparecem.

```bash
git fetch origin
git status --short                       # o que mudou no Mac
git checkout -b iphone-mac               # um ramo só para isto (as mudanças vêm atrás)
git checkout origin/site-sealer -- .gitignore   # .gitignore novo, que esconde os segredos
git add ios scripts .gitignore
git status --short                       # NÃO pode aparecer GoogleService-Info.plist nem .p8
git commit -m "iPhone 0.9.3: codigo do Mac (grupos, voz, ficheiros, notificacoes)"
git rebase origin/site-sealer            # põe-no por cima do núcleo atual
git push origin iphone-mac
```

Depois de rever o ramo `iphone-mac`, junta-se à `site-sealer` (`git checkout site-sealer && git merge
--ff-only iphone-mac && git push origin site-sealer`).
