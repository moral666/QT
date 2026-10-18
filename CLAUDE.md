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

- **iOS fica de lado por agora** (tratar depois). A app iPhone está atrás da
  Android (sem convites, reações, voz, grupos nem presença).
- **Sem avisos de ligação na app** ("A ligar ao relay", "Sem ligação",
  barras de estado): se houver problemas, só no registo (`adb logcat -s SealerRelay`).
- A build de release **não mostra opções de teste do relay** (URL, chave).
- O **servidor fica em Estocolmo** (AWS eu-north-1). Não propor mudar de região.
- O **site nunca menciona "sem auditoria externa"**.
- README: não falar da mudança de critérios dos tempos (ms) da página de estado.
- Versões da app: série 0.x (agora **0.9.1**, `versionCode` 8), em
  `android/app/build.gradle.kts`. **Cada mudança na app sobe o último número** (0.9.1, 0.9.2,
  0.9.3...) e o `versionCode` mais 1, no mesmo commit. Só se passa a **1.0.0** quando o Nuno disser.
- Conta de programador da Google Play (25 USD): só depois de testar a app. Conta pessoal ou de
  empresa: decide ele mais tarde.
- **Identificador da app: `sealer.app`** (desde a 0.9.0; antes `pt.qt.app`). Nunca mudar depois de
  publicar. O `namespace` e o pacote Kotlin continuam `pt.qt.app`.
- **Notificações: FCM, opção 1** (17/10/2026): campainha vazia e aviso genérico "Tens mensagens
  novas", sem decifrar nada em segundo plano. Nome/texto na notificação (opção 2) fica para depois.
  App: `Push.kt` lê `android/app/google-services.json` (fora do git). Relay: `/etc/qt/fcm.json`
  (conta de serviço, fora do git) e o `setup-ec2.sh` liga as notificações se o ficheiro existir.
  Só tocam para mensagens: recibos, reações, convites e presença vão com `silent: true` (18/10/2026).

## O projeto

- QT é o protocolo e o repositório; as apps chamam-se **SEALER**. Site em
  `site/` (sealer.pt, Cloudflare Worker com deploy automático a partir do git).
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
6 mensagem de grupo, 7 info do grupo, 8 saída do grupo, 9 presença, 10 sessão terminada. O próximo
tipo livre é o 11. Uma versão antiga mostra "atualiza a app" para tipos novos.
Ficheiros: tipo 3 com o corpo na versão 2 (com o nome); imagens e voz na versão 1.

### Funcionalidades da 0.8.0

Bloquear contactos, termos de utilização, apagar a conta também no servidor,
convite (Aceitar / Recusar / Bloquear), reações, voz até 10 s, grupos até 10
pessoas com 3 administradores (envio a cada membro, `ffi/src/groups.rs`; criar
não exige pessoas nem nome), online / offline só para contactos e cifrado
(`ffi/src/presence.rs`, interruptor no menu), ecrã do ID só com QR (em
maiúsculas, menos denso) e ID abreviado.

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
./ffi/generate_bindings.sh              # gera os bindings e corre os testes em Python
```

### Build automática (GitHub Actions)

`.github/workflows/android.yml` corre a cada push que mexa em `android/` ou no Rust:
gera os bindings, `gradle :app:testDebugUnitTest`, `:app:assembleDebug`,
`:app:minifyReleaseWithR8` e `:app:lintDebug`. Ver com `gh run list -w Android` e
`gh run view <id> --log-failed`. É a forma de apanhar erros de Kotlin antes da build do Nuno.

### Build e instalação no PC do Nuno (pasta `~/QT-repo`)

Se o Rust mudou (núcleo, ffi, storage):
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

- `wss://13-51-46-213.sslip.io`, pin `f018d46b67d2437571cefc86827dec0dc3b0fd586bd22c664d3839f6a1c67f1a`
  (a build lê `qtRelayUrl` / `qtRelayPin` das propriedades do Gradle).
- Atualizar o servidor (no EC2):
  ```
  sudo git -C /opt/qt-src fetch --depth 1 origin site-sealer && sudo git -C /opt/qt-src reset --hard FETCH_HEAD && sudo QT_BRANCH=site-sealer bash /opt/qt-src/deploy/aws/setup-ec2.sh
  ```
- O Redis de produção tem uma ACL com poucos comandos (ver `server/tests/redis_acl.rs`):
  sem SETEX nem EXISTS (usar SET + EXPIRE e GET).
- Estado e tempos públicos: `https://sealer.pt/api/estado` (Worker em `site/worker/`).
- TLS no servidor: **HAProxy + certbot** (desde 18/10/2026; antes Caddy, que gastava ~137 KB por
  ligação). Sem registos de acesso. Limite medido com o Caddy: ~8 000 ligações (2 por app aberta).

## Ambiente do Claude (nuvem)

- Não há rede para o Maven: não se consegue compilar a app Android aqui. Para
  apanhar erros de sintaxe em Kotlin, usar o `kotlinc` (descarregado do GitHub)
  e procurar "expecting|syntax|unexpected|unclosed". A build do Nuno é o teste real.
- `curl` para sealer.pt e sslip.io é bloqueado: usar o WebFetch.
- Não usar `pkill -f` (mata a própria shell).
- Às vezes o crates.io e o push para o GitHub estão bloqueados na nuvem: aí o Rust não compila
  aqui (só `rustfmt --check` para a sintaxe) e o Nuno faz o commit e o push no PC; a CI compila e testa.

## Por decidir (perguntar quando fizer sentido)

- O site ainda diz "versão 0.7" (topo e quadrado "O que é a versão 0.7"): passar a 0.9?
- Código do ID mais curto: A) mesmo ID noutra escrita (~92 caracteres, só núcleo),
  B) código curto no servidor que expira (precisa de mudar o relay),
  C) ID com metade do tamanho (mudança grande, os IDs atuais deixam de servir).
  Sugestão dada: deixar como está.
- Depois: atualizar a app iPhone (inclui notificações APNs), AAB e ficha da Play Store.
