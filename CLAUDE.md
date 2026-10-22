# Notas de trabalho (para o Claude)

Este ficheiro é lido no início de cada conversa com este repositório. Diz como o
Nuno Gonzalez (dono do projeto, desenvolve sozinho) quer que se trabalhe e onde
está cada coisa. Mantém-no atualizado quando algo mudar.

## Como falar e trabalhar com o Nuno

- Responder em **português de Portugal**, com **respostas curtas** e diretas.
- **Nada de travessões (o traço comprido)** na documentação nem no site: usar dois pontos,
  vírgulas, parênteses ou frases separadas.
- **Aprovação obrigatória:** qualquer mudança significativa na arquitetura do
  projeto ou introdução de SDKs proprietários (Firebase/FCM, etc.) tem de ser
  aprovada manualmente por ele. Apresentar as soluções de forma modular,
  explicando o impacto técnico de cada escolha **antes** de gerar o código.
- **Auditorias / revisões de código:** para cada problema encontrado, explicar
  de forma direta O QUE está incorreto, COMO propões corrigir e o impacto no
  comportamento da app, e PARAR a pedir aprovação explícita. Só escrever a
  correção depois de ele responder.
- Ele testa as builds nos telemóveis (2 Galaxy S23 e 1 S26) e no emulador. No
  fim de cada mudança, dar-lhe os passos exatos de build e instalação (abaixo).
- Fazer commit e push no fim de cada tarefa (ramo `site-sealer`).

## Decisões que não se mudam sem ele pedir

- **App iPhone** (posta a par na 0.9.13, 19/10/2026; **iPhone 0.9.15 + Android 0.9.16 testados juntos a 20/10/2026, sem erros**,
  incluindo chamadas Android/iPhone/iPhone): posta a par da Android 0.9.13
  pelo Claude (o código Swift da 0.9.3 do Mac nunca chegou ao GitHub). Identificador `sealer.app`.
  Decisões tomadas pelo Claude com as regras do Nuno (privacidade e desempenho), a confirmar com ele:
  **notificações direto pela Apple, sem Firebase** (`server/src/apns.rs`, `/etc/qt/apns.json`, token
  `apns:...`), **chamadas com CallKit + PushKit** (fora do histórico do telefone), Face ID no bloqueio
  da app, a escolha do idioma nas Definições do iPhone, fotos de perfil em JPEG. Textos:
  `python3 scripts/strings-ios.py` (a partir dos do Android). Ver `ios/README.md`. Sem conta paga da
  Apple não há notificações (`QT_ENTITLEMENTS` no `ios/Configs/Local.xcconfig`).
- **Sem avisos de ligação na app** ("A ligar ao relay", "Sem ligação",
  barras de estado): se houver problemas, só no registo (`adb logcat -s SealerRelay`).
- A build de release **não mostra opções de teste do relay** (URL, chave).
- O **servidor fica em Estocolmo** (AWS eu-north-1). Não propor mudar de região.
- O **site, a documentação e a app nunca mencionam auditorias em falta** ("sem auditoria", "não auditada", 19/10/2026).
- README: não falar da mudança de critérios dos tempos (ms) da página de estado.
- Versões da app: série 0.x (agora **0.9.17**, `versionCode` 24), em
  `android/app/build.gradle.kts`. **Cada mudança na app sobe o último número** (0.9.1, 0.9.2,
  0.9.3...) e o `versionCode` mais 1, no mesmo commit. Só se passa a **1.0.0** quando o Nuno disser.
- Conta de programador da Google Play (25 USD): só depois de testar a app. Conta pessoal ou de
  empresa: decide ele mais tarde.
- **Identificador da app: `sealer.app`** (desde a 0.9.0; antes `pt.qt.app`). Nunca mudar depois de
  publicar. O `namespace` e o pacote Kotlin continuam `pt.qt.app`.
- **Notificações: FCM, opção 1** (17/10/2026): campainha vazia e aviso genérico "Tens mensagens
  novas". Nome/texto na notificação (opção 2) fica para depois. **Desde a 0.9.6 (chamadas 4b,
  19/10/2026)** a app decifra em segundo plano (`SealerCore` em `Core.kt`, um só `QtClient` por
  processo) e a chave da base de dados funciona com o telemóvel bloqueado (Keystore
  `qt_wrap_key_bg_v1`, formato "QTK3"); a notificação das mensagens continua genérica. As chamadas
  levam a campainha marcada (`wake`, `{"qt":"call"}` / `"call_end"`): o Nuno aceitou que o relay e
  a Google saibam o momento de uma chamada, para tocar "Chamada SEALER" sem nome com o bloqueio
  da app ativo.
  App: `Push.kt` lê `android/app/google-services.json` (fora do git). Relay: `/etc/qt/fcm.json`
  (conta de serviço, fora do git) e o `setup-ec2.sh` liga as notificações se o ficheiro existir.
  Só tocam para mensagens: recibos, reações, convites e presença vão com `silent: true` (18/10/2026).
  iPhone: sem Firebase, o relay fala com a Apple (`server/src/apns.rs`) se existir `/etc/qt/apns.json`
  (`team_id`, `key_id`, `topic` `sealer.app`, `private_key` da chave `.p8`; fora do git e da conversa).

## O projeto

- QT é o protocolo e o repositório; as apps chamam-se **SEALER**. Site em
  `site/` (sealer.pt, Cloudflare Worker com deploy automático a partir do git).
- **Site em PT, EN e ES** (19/10/2026, escolha do Nuno: trocar o texto com JavaScript, uma só página).
  O português fica no HTML; cada texto tem `data-i18n="pagina.n"` (atributos: `data-i18n-alt`,
  `-aria-label`, `-content`) e as traduções estão em `site/src/i18n/en.js` e `es.js` (carregadas só
  nessas línguas). Textos criados em JS: `L({ pt, en, es })` de `src/i18n.js`. Língua: `?lang=` no
  endereço (seletor PT · EN · ES) ou a do navegador (outra qualquer: inglês). **Nada é guardado no
  navegador** (a política de cookies diz que não há armazenamento). Ao mudar um texto do site,
  mudar também em `en.js` e `es.js`. As páginas legais traduzidas dizem que vale a versão portuguesa.
- App iPhone: `ios/` (SwiftUI, mesmo núcleo). Ao mudar um ecrã no Android, mudar também no iPhone
  (tabela em `ios/README.md`); ao mudar um texto, correr `python3 scripts/strings-ios.py`.
- Núcleo Rust: `core` (PQXDH + Double Ratchet + Sealed Sender), `storage`
  (SQLCipher), `ffi` (UniFFI para Kotlin/Swift), `transport` (WebSocket + Noise),
  `server` (relay, Redis). Nenhuma criptografia em Kotlin.
- App Android: `android/` (Kotlin + Compose). Ficheiros principais:
  `MainViewModel.kt`, `MainActivity.kt`, `Groups.kt`, `Voice.kt`, `Attachments.kt`,
  `Files.kt` (ficheiros), `Composer.kt` (caixa de escrita, igual a dois e nos grupos),
  `RelayLink.kt`.
- Documentos de referência: `docs/protocol-spec.md` (formato de tudo o que vai
  na rede), `docs/known-limitations.md`, `docs/google-play.md`,
  `docs/export-compliance.md`, `CHANGELOG.md`.

### Mensagens dentro do ratchet (payload v1)

`[1 versão=0x01][1 tipo][16 msg_id][8 sent_at]` e o corpo. Tipos: 1 texto,
2 recibo, 3 anexo (imagem, GIF ou voz `audio/mp4`), 4 convite, 5 reação,
6 mensagem de grupo, 7 info do grupo, 8 saída do grupo, 9 presença, 10 sessão terminada, 11 chamada
de voz, 12 foto de perfil (`ffi/src/profile.rs`, 0.9.9: WebP de 72 em 72 h, só contactos; desde a 0.9.11
até 9 973 bytes, para o payload caber nos 10 000). 13 badge (`ffi/src/badges.rs`, 0.9.14: certificado do emissor, ver `docs/protocol-spec.md` secção 20). O próximo tipo livre é o 14. Uma versão antiga mostra "atualiza a app" para tipos novos.
Ficheiros: tipo 3 com o corpo na versão 2 (com o nome); imagens e voz na versão 1.

### Funcionalidades da 0.8.0

Bloquear contactos, termos de utilização, apagar a conta também no servidor,
convite (Aceitar / Recusar / Bloquear), reações, voz até 10 s, grupos até 10
pessoas com 3 administradores (envio a cada membro, `ffi/src/groups.rs`; criar
não exige pessoas nem nome), online / offline só para contactos e cifrado
(`ffi/src/presence.rs`, interruptor no menu), ecrã do ID só com QR (em
maiúsculas, menos denso) e ID abreviado.

### Segurança da 0.9.3 (só núcleo Rust e relay; o Kotlin não mudou)

PQXDH v2 (transcrição no HKDF), bundle v4 (spk_id + data assinada, recusado com mais de 90
dias), rotação da signed pre-key e da prekey ML-KEM a cada 7 dias (`ffi/src/prekeys.rs`),
one-time prekeys servidas pelo relay (lote diário de 100 no `RegisterPreKeyBundle`, uma por
`FetchPreKeyBundle`), envelope v3 com enchimento (256/1024/4096/10 240), prova de trabalho
Argon2id para contas novas (`core/src/pow.rs`, 7 bits) e limites por conta no relay (300
envios/min, 60 bundles/min). As apps 0.9.3 não falam com apps 0.9.2: atualizar todos os
telemóveis e o relay primeiro.

### Chamadas de voz (em curso, aprovado a 19/10/2026)

Só Android primeiro, 1 a 1, áudio sempre pelo relay (sem P2P). Quatro fases, cada uma aprovada
antes: 1) núcleo, sinalização (tipo 11) e cifra dos pacotes estilo WireGuard (**feita**,
`core/src/call.rs`, `ffi/src/call.rs`, `ffi/tests/calls.rs`); 2) relay UDP 443 com salas
anónimas e reserva por WebSocket sem conta (**feita**, processo à parte `qt-call-relay`,
`server/src/call_relay.rs`; porta 443 UDP aberta no Security Group a 19/10/2026; testado em
produção a partir de casa do Nuno: ida e volta PC-relay-PC-relay-PC de 128 ms); 3) motor de áudio
(**feita**, crate `call/` = `qt_call`: libopus 1.6.1 com DRED e PLC neural pelo `opusic-sys`, jitter
buffer adaptativo, rede com reserva WebSocket; ferramenta `tools/call-demo`); 4) Android, decidido a 19/10/2026: 1B (a chave da
base de dados passa a funcionar com o telemóvel bloqueado), 2A (reserva WebSocket no Rust, com
rustls), só tocam contactos guardados, e em duas partes: **4a feita** (app 0.9.4: chamadas com a
app aberta, `Calls.kt`, `CallScreen.kt`, `CallAudio` no `ffi/src/call_audio.rs`, feature `calls`;
serviço em primeiro plano do tipo microphone) e **4b feita, a testar** (app 0.9.6: notificação de
ecrã inteiro com Atender/Recusar, ecrã da chamada por cima do bloqueio do telemóvel, `Core.kt`,
`CallNotify`, `CallActionReceiver`; com o bloqueio da app toca "Chamada SEALER" e pede o código).
**ConnectionService (3A) adiado** pelo Nuno a 19/10/2026: o Telecom passa a gerir o modo de áudio, o
foco e o altifalante, e isso choca com o áudio atual; fazer à parte, com testes dele. 0.9.5: jitter
buffer espera até 200 ms por tempo e recomeça depois de uma paragem do outro lado (antes ficava
mudo); modo de chamada pedido antes de criar o microfone.
Ver `docs/protocol-spec.md`, secção 18.

### Design (desde a 0.9.11, aprovado pelo Nuno a 19/10/2026)

Maqueta no canvas "Sealer: novo design" (claude.ai). Cabeçalhos com o fundo curvo
(`CurvedBottomShape` e `GalaxyHeader` em `Design.kt`; a `SealerTopBar` usa-o em todos os ecrãs).
**O logo só roda (pião, `BrandMark`) no cabeçalho do ecrã inicial**; nos outros ecrãs fica parado
(`BrandLogo`). Ecrã inicial sem menu de três pontos: engrenagem na minha foto abre as Definições
(`ProfileSettingsScreen`) e o "+" abre o Novo contacto (`AddContactScreen`, também "Novo grupo").
A app não tem nome próprio do utilizador: o cabeçalho mostra "Conversas" e o ID abreviado. Letra:
Plus Jakarta Sans (`res/font/`, `JakartaSans` no `Theme.kt`); "Sealer" continua em Manrope Bold.
Foto de perfil: escolher, ver e só depois "Aplicar" (`previewProfilePhoto` / `applyProfilePhoto`);
original até 20 MB. Imagens do design no site em `site/public/ecras/` (geradas da maqueta).

### Idiomas (desde a 0.9.10)

PT (base, `res/values/strings.xml`), EN, FR, ES, DE e IT (`res/values-xx/`). **Nenhum texto do ecrã
escrito no Kotlin:** usar `s(R.string.chave, args...)` (`Lang.kt`, funciona em ecrãs, ViewModel e
notificações) e acrescentar a chave nas seis línguas. Segue a língua do telemóvel (PT se não for
nenhuma) e há o menu "Idioma" (Android 13+: `LocaleManager`, também nas definições da app; antes,
`Lang.localized` e `recreate()`). Textos que vêm do Rust (📷 Foto, voz, "atualiza a app") passam
por `coreText()`. Registos (`Log`) e mensagens de exceções continuam em português.

### Badges (desde a 0.9.14, aprovado pelo Nuno a 20/10/2026)

Aro colorido à volta da foto (Founder dourado, Co-founder rosa, Dev preto, Tester ciano,
Contributor verde), sem nada guardado no servidor e sem P2P. Certificado Ed25519 do emissor
(`core/src/badge.rs`), entregue pela ferramenta `cargo run --release -p qt_server --example badge`
(no PC do Nuno, chave do emissor fora do git e do servidor) e enviado pela app aos contactos no tipo 13.
Um só desenho (`res/drawable-nodpi/badge_ring.png`, `Assets.xcassets/BadgeRing`) pintado com duas
cores (`Badges.kt`, `Views/Badges.swift`). Enquanto `ISSUER_PUBLIC_KEY` estiver a zeros, nenhum badge
é aceite. Desde a 0.9.16 há também a medalha do badge no canto da foto (`badge_coin_*.png`,
`BadgeCoin1..5`, recortadas da folha de badges do Nuno): em baixo à direita nos contactos (o ponto
online passa para cima) e em baixo à esquerda na minha foto do cabeçalho.

### Funcionalidades da 0.9.0

Ficheiros até 25 MB (PDF, documentos...) a dois e em grupos, "Abrir" noutra app (lê da
memória, sem cópia em disco) ou "Guardar"; fotos, GIFs e voz também nos grupos (o ficheiro
cifrado vai uma vez ao relay; nos grupos quem recebe não o apaga, o relay apaga ao fim de 24 h).

## Comandos

### Testes (na nuvem do Claude ou no PC)

```
cargo test -p qt_ffi --features pq      # núcleo da app (inclui grupos e presença)
cargo test -p qt_storage
cargo test -p qt_server                 # precisa de Redis local: redis-cli ping || redis-server --daemonize yes
cargo test -p qt_call                   # motor das chamadas (precisa do cmake: compila o libopus)
./ffi/generate_bindings.sh              # gera os bindings e corre os testes em Python
```

### Testar as chamadas no PC (ferramenta de eco)

Precisa de `sudo apt install cmake libasound2-dev` e de auscultadores:
```
cd ~/QT-repo/tools/call-demo && cargo run --release -- eco            # voz -> relay -> auscultadores
cargo run --release -- eco --perda 15                                # 15% de pacotes perdidos
cargo run --release -- eco --ws                                      # reserva wss://.../call
cargo run --release -- eco --tom                                     # som de teste, sem microfone
```

### Build automática (GitHub Actions)

`.github/workflows/android.yml` corre a cada push que mexa em `android/` ou no Rust:
gera os bindings, `gradle :app:testDebugUnitTest`, `:app:assembleDebug`,
`:app:minifyReleaseWithR8` e `:app:lintDebug`. Ver com `gh run list -w Android` e
`gh run view <id> --log-failed`. É a forma de apanhar erros de Kotlin antes da build do Nuno.
O `ci.yml` corre `cargo clippy --workspace -- -D warnings` com o Rust **stable** mais recente: cada
versão nova do clippy pode trazer avisos novos que partem a CI (19/10/2026: `chunks_exact` com
tamanho fixo, trocado por `as_chunks`). O núcleo precisa de Rust 1.88+.

### Build e instalação no PC do Nuno (pasta `~/QT-repo`)

Se o Rust mudou (núcleo, ffi, storage, call). Precisa do `cmake` (Opus das chamadas):
```
cd ~/QT-repo && git pull
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/30.0.16248370
ABIS="arm64-v8a armeabi-v7a x86_64" ./scripts/build-android.sh
cd android && ./sync-native.sh
```
Depois, no Android Studio: Sync e, com Ctrl Ctrl, `gradle assembleRelease`
(confirmar a hora do APK com `ls -l`; "up-to-date" pode querer dizer APK antigo).
Instalar:
```
~/Android/Sdk/platform-tools/adb install -r ~/QT-repo/android/app/build/outputs/apk/release/app-release.apk
```
Se só mudou Kotlin, basta `git pull`, Sync, `assembleRelease` e instalar.
Notas: o NDK r26d não compila a x86_64 (o emulador precisa dela, usar o NDK 30);
o `adb` não está no PATH (usar o caminho completo); não há `gradlew` no repo.

### Relay (AWS EC2, eu-north-1)

- `wss://relay.sealer.pt` (desde a app 0.9.2, 19/10/2026), pin `f018d46b67d2437571cefc86827dec0dc3b0fd586bd22c664d3839f6a1c67f1a`
  (a build lê `qtRelayUrl` / `qtRelayPin` das propriedades do Gradle). O nome é um registo A na
  Cloudflare **só DNS (nuvem cinzenta, sem proxy, por privacidade)**, TTL 1 dia, a apontar para o IP
  elástico 13.51.46.213. Antes de mudar de servidor, baixar o TTL um dia antes. Desde 19/10/2026 o
  servidor só serve `relay.sealer.pt` (o `13-51-46-213.sslip.io` das apps até à 0.9.1 foi desligado,
  com aprovação do Nuno); o site e a configuração do iPhone também usam o nome novo. Nomes em
  `/etc/qt/domains`.
- **Chave do relay:** cópia no PC do Nuno (`~/Documentos/SENSIVEL /`), nunca no git nem no projeto.
  Passou pela conversa a 19/10/2026: trocar por uma nova (pin novo na app) antes da Play Store.
- Atualizar o servidor (no EC2):
  ```
  sudo git -C /opt/qt-src fetch --depth 1 origin site-sealer && sudo git -C /opt/qt-src reset --hard FETCH_HEAD && sudo QT_BRANCH=site-sealer bash /opt/qt-src/deploy/aws/setup-ec2.sh
  ```
- **Relay das chamadas** (`qt-call-relay`, desde a fase 2): UDP 443 público e reserva em
  `wss://relay.sealer.pt/call` (HAProxy para 127.0.0.1:9445). Usa a mesma chave Noise (o pin),
  não tem acesso ao Redis nem a `/etc/qt`. Registo: `sudo journalctl -u qt-call-relay -f`.
- O Redis de produção tem uma ACL com poucos comandos (ver `server/tests/redis_acl.rs`):
  sem SETEX nem EXISTS (usar SET + EXPIRE e GET).
- Estado e tempos públicos: `https://sealer.pt/api/estado` (Worker em `site/worker/`).
- TLS no servidor: **HAProxy + certbot** (desde 18/10/2026; antes Caddy, que gastava ~137 KB por
  ligação). Sem registos de acesso. Limite medido com o Caddy: ~8 000 ligações. Cada app aberta usa **3 ligações** (pedidos, espera e
  anexos, `MainViewModel.startLink`), mais 1 de curta duração quando a campainha acorda o segundo plano.

## Ambiente do Claude (nuvem)

- Não há rede para o Maven: não se consegue compilar a app Android aqui. Para
  apanhar erros de sintaxe em Kotlin, usar o `kotlinc` (descarregado do GitHub)
  e procurar "expecting|syntax|unexpected|unclosed". A build do Nuno é o teste real.
- `curl` para sealer.pt e sslip.io é bloqueado: usar o WebFetch.
- Não usar `pkill -f` (mata a própria shell).
- Às vezes o crates.io e o push para o GitHub estão bloqueados na nuvem: aí o Rust não compila
  aqui (só `rustfmt --check` para a sintaxe) e o Nuno faz o commit e o push no PC; a CI compila e testa.
  O Nuno ligou "Allow network egress" a 19/10/2026: com isso o crates.io funciona.
- Na nuvem há `cmake`; o ALSA para a ferramenta `tools/call-demo` instala-se com
  `apt-get update && apt-get install -y libasound2-dev`. A nuvem não chega ao relay por UDP (só DNS):
  testar a produção a partir do PC do Nuno.
- Na nuvem não há `libsqlcipher`: compilar e testar com o SQLCipher embutido, por exemplo
  `cargo test -p qt_ffi --features "pq qt_storage/bundled-sqlcipher"` e
  `cargo test -p qt_server -p qt_storage --features qt_storage/bundled-sqlcipher`.

## Por decidir (perguntar quando fizer sentido)

- Código do ID mais curto: A) mesmo ID noutra escrita (~92 caracteres, só núcleo),
  B) código curto no servidor que expira (precisa de mudar o relay),
  C) ID com metade do tamanho (mudança grande, os IDs atuais deixam de servir).
  Sugestão dada: deixar como está.
- Depois: AAB e ficha da Play Store; ficha da App Store (conta paga da Apple).
