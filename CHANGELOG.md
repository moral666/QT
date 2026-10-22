# Changelog

Formato inspirado em [Keep a Changelog](https://keepachangelog.com/).
Datas aproximadas: este projeto ainda não segue versionamento semântico
formal (não há releases numeradas ainda, ver `ROADMAP.md`).

## [Não lançado]

### 22 de outubro de 2026: site revisto contra o código

- Site (6 línguas): textos acertados com o que o código faz. O servidor "não guarda" (em vez de
  "não sabe") quem envia e quem denuncia; as apps usam sempre o PQXDH; o texto vai em 4 tamanhos
  fixos; a fila de mensagens apaga-se 30 dias depois da última; nos grupos os ficheiros ficam até
  às 24 horas; a foto de perfil fica cifrada na fila até o contacto a receber; diferenças do iPhone
  (fotos só da galeria, idioma nas Definições do iPhone, notificações quando chegar à App Store);
  recibos só nas conversas a dois; data do teste dos 5 aparelhos.
- Termos: o serviço inclui voz, ficheiros, grupos e chamadas; todas as funções gratuitas durante os
  testes, com funções que poderão vir a ser pagas (as mensagens de texto ficam sempre grátis);
  idade mínima de 16 anos; licença AGPL-3.0 ou posterior, com licença comercial à parte.
- Política de privacidade: o acesso às funções pagas guardado no servidor e durante quanto tempo.
- Modelo de ameaças, ROADMAP e limitações conhecidas (ponto 33) acertados da mesma forma.

### 22 de outubro de 2026: site e documentação

- Site: página nova **Funcionalidades** (`funcionalidades.html`, PT, EN e ES) com tudo o que a app
  faz; botão "O que a app faz" no início e ligações no rodapé. Versões no topo: Android 0.9.22 e
  iPhone 0.9.22.
- Política de privacidade, termos e "Apagar a conta" atualizados: cópia de recuperação, denúncias,
  contas banidas, medalhas e o caminho novo para as Definições.
- Documentação revista: README (PT e EN), ROADMAP, modelo de ameaças, limitações conhecidas
  (medalhas, denúncias, chave de recuperação), SECURITY e READMEs das apps.

### 22 de outubro de 2026: revisão das apps (0.9.22, Android e iPhone)

- Bloqueio da app: a espera depois de 5 códigos errados conta pelo relógio desde que o telemóvel
  ligou, que não se muda nas definições. Mudar a hora já não salta a espera; depois de reiniciar o
  telemóvel a espera recomeça inteira. A tentativa errada fica gravada antes de aparecer "código errado".
- Android 7 a 12: a miniatura na lista de apps recentes já não mostra as conversas (como no
  Android 13+ e no iPhone). Com a app aberta as capturas de ecrã continuam permitidas.
- Depois de recuperar a conta, o meu badge, as fotos e a presença aparecem logo, sem reiniciar a app.
- Android: "A enviar…" e "A receber…" dos ficheiros traduzidos nas 6 línguas; leituras da base de
  dados fora das atualizações do estado do ecrã.

### 22 de outubro de 2026: premium controlado pelo operador (apps 0.9.21, Android e iPhone)

- Quem tem premium e se o bloqueio está ligado passa a ser decidido só no relay, pela aba "Premium"
  da ferramenta do operador: bloqueio para todos, dar, tirar e lista.
- Com o bloqueio ligado, o relay recusa anexos e ofertas de chamada de contas sem premium
  (`AdminPremiumEnforce`, `PREMIUM_REQUIRED`), mesmo de uma app alterada.
- As apps perguntam ao relay antes de cada ação paga: tirar o premium conta logo. Saíram o
  interruptor de teste e o "simular premium".
- Preço: 5 €/ano.

### 22 de outubro de 2026: Sealer Premium (apps 0.9.20, Android e iPhone)

- Ecrã "Sealer Premium" (12 €/ano, 5 €/ano desde a 0.9.21): o premium desbloqueia fazer chamadas, criar grupos e enviar
  fotos, GIFs, voz e ficheiros. Texto grátis para sempre; sem premium recebe-se tudo.
- Bloqueio com interruptor de teste, até haver pagamentos; com ele ligado, uma ação paga sem
  premium abre o ecrã Premium.
- O relay guarda até quando cada conta tem premium (`PremiumStatus`, `AdminGrantPremium`,
  `AdminListPremium`). O operador dá ou tira premium (testadores, ofertas) pela ferramenta
  `denuncias`.

### 22 de outubro de 2026: chave de recuperação da conta (apps 0.9.19, Android e iPhone)

- Definições → **Chave de recuperação**: 12 palavras (mostradas uma vez, com confirmação de 3).
  Num telemóvel novo, "Já tenho conta: recuperar com as 12 palavras" traz o mesmo ID, os
  contactos, os bloqueados e o badge. O histórico e os grupos não voltam.
- Cópia cifrada na app com uma chave derivada das palavras (Argon2id + XChaCha20-Poly1305); o
  relay guarda-a sem a conseguir ler (`BackupPut`, `BackupGet`, `BackupDelete`), 90 dias
  renovados quando a app abre. É atualizada sozinha quando os contactos mudam.
- Depois de recuperar, as sessões com os contactos recomeçam sozinhas (convite e sessão nova).
- Contadores novos no relay: `copias_guardadas` e `copias_pedidas`.

### 22 de outubro de 2026: denunciar contactos e banir contas (apps 0.9.18, Android e iPhone)

- Menu da conversa → **Denunciar**: a pessoa escolhe o motivo e as mensagens a incluir (até 20).
  A denúncia é cifrada na app para a chave do operador; o relay guarda-a 30 dias sem a conseguir
  ler e sem o ID de quem denunciou. Depois de enviada, o contacto fica bloqueado.
- Relay: `SubmitReport` (até 10 por conta e por dia) e pedidos de operador (`AdminListReports`,
  `AdminDeleteReport`, `AdminBan`, `AdminUnban`, `AdminListBans`), aceites só da chave em
  `/etc/qt/operator.pub`. Conta banida: o relay apaga o que guarda dela e deixa de a aceitar.
  Contadores novos: `denuncias` e `banidas`.
- Ferramenta do operador `server/examples/denuncias.rs` (chave, listar, apagar, banir, desbanir,
  banidos).
- A opção só aparece nas apps depois de a chave pública do operador estar em
  `OPERATOR_REPORT_PUBLIC` (`core/src/report.rs`).

### 22 de outubro de 2026: revisão de segurança do núcleo (apps 0.9.17, Android e iPhone)

- Chave da base de dados (SQLCipher): o texto com a chave já não é copiado a meio, por isso não
  ficam pedaços da chave na memória libertada.
- Ratchet: os valores intermédios das chaves (HKDF, DH, chave de cada mensagem enviada) são
  apagados da memória logo depois de usados. Nada muda nas mensagens: testes com valores do código
  anterior confirmam que as sessões guardadas e as apps antigas continuam a funcionar.
- Um erro interno (panic) no Rust já não deixa a app a dar "erro de armazenamento" até ser
  reiniciada.
- Fuzzing local (`fuzz/`): 5 alvos, nenhum erro na primeira volta.

### 22 de outubro de 2026: relay só aceita escritas de contas registadas

- Uma chave criada no momento (sem bundle nem prova de trabalho) conseguia pôr mensagens na fila de
  IDs que nunca existiram, guardar tokens de notificações e criar anexos. Num teste local com a
  configuração do Redis de produção (64 MB), 20 ligações encheram o Redis em 45 s e a seguir nem
  contas normais conseguiam registar-se. Agora `SendMessage` exige bundle de quem envia e de quem
  recebe; `RegisterPushToken` (com token) e `BlobCreate` exigem bundle de quem pede. As apps não
  mudam: o que for recusado fica na outbox e segue no ciclo seguinte.
- Ferramenta `server/examples/abuso.rs` (teste de abuso controlado) e testes de regressão em
  `server/tests/relay_registo.rs`. Contadores novos: `sem_registo` e `destinatario_desconhecido`.

### 20 de outubro de 2026: teste entre Android 0.9.16 e iPhone 0.9.15

- Testado sem erros: contactos (adicionar, apagar, voltar a adicionar), QR, partilha do ID, menus,
  texto, voz, GIFs, ficheiros, bloquear, apagar conversas e chamadas de voz entre Androids, entre
  Android e iPhone e entre iPhones. README, roadmap, documentação e site atualizados com este estado.

### 20 de outubro de 2026: medalha dos badges (apps 0.9.16)

- Além do aro, quem tem badge mostra a medalha (moeda com o símbolo) no canto da foto: lista de
  conversas, cabeçalho da conversa, chamadas, grupos e a minha foto no ecrã inicial. Com medalha,
  o ponto "online" passa para o canto de cima.

### 20 de outubro de 2026: botão de ligar e convites (app 0.9.15, Android e iPhone)

- Quem me escreveu primeiro (convite) e eu adicionei à mão pelo ID ou pelo QR ficava com o convite
  pendente: a conversa escondia o botão de ligar. Agora adicionar resolve o convite, e a conversa só
  conta como convite se a pessoa não estiver nos contactos.
- iPhone: áudio das chamadas mais robusto (microfone por "tap", o áudio é ativado pela app se o
  CallKit não avisar) e registo das chamadas na consola do Xcode (`SealerCall`).

### 20 de outubro de 2026: badges (apps Android e iPhone 0.9.14)

- **Badges:** Founder, Co-founder, Dev, Tester e Contributor. Quem tem um aparece com um aro da cor
  do badge à volta da foto (lista de conversas, conversa, chamadas, grupos e a minha foto).
- **Sem nada no servidor e sem P2P:** o badge é um certificado assinado pelo emissor
  (`core/src/badge.rs`), entregue cifrado pela ferramenta `server/examples/badge.rs` e enviado pela
  app aos seus contactos (tipo 13). Só vale para a identidade de quem o tem; retirar é um
  certificado novo com o badge 0.
- **Aro:** um só desenho (o aro dourado em tons de cinza), pintado em cada cor no telemóvel.

### 19 de outubro de 2026: iPhone igual ao Android (app iPhone 0.9.13, relay)

- **App iPhone posta a par da Android 0.9.13** (o código do GitHub só tinha conversas a dois, fotos e
  GIFs): grupos, voz, ficheiros, reações, bloquear, convites, online, apagar a conta também no
  servidor, foto de perfil, termos de utilização, bloqueio da app (código e Face ID), as seis
  línguas (geradas a partir das do Android por `scripts/strings-ios.py`), o novo design (0.9.11) e
  as chamadas de voz (CallKit e PushKit, sem aparecer no histórico do telefone). Identificador
  `sealer.app`, como no Android. O aviso antigo do ecrã inicial saiu.
- **Notificações do iPhone sem Firebase:** o relay fala diretamente com a Apple
  (`server/src/apns.rs`, token `apns:...`, `/etc/qt/apns.json` no `setup-ec2.sh`). A Apple recebe
  um aviso vazio com o texto fixo da app; as chamadas vão por push VoIP.
- **Núcleo:** a build do iPhone passa a incluir as chamadas (`ios = [..., "calls"]`); o
  `build-ios.sh` pede o cmake.
- **Site:** política de privacidade com a Apple (só no iPhone), em PT, EN e ES.

### 19 de outubro de 2026: CI reparada, texto do ecrã inicial e documentação revista (app 0.9.13)

- **CI reparada.** O clippy novo do Rust (1.99) passou a recusar `chunks_exact` com um tamanho
  fixo (`ffi/src/prekeys.rs`). Trocado por `as_chunks`, que faz o mesmo e é verificado pelo
  compilador; também em `ffi/src/call_audio.rs` e `server/tests/call_audio_ffi.rs`. O núcleo
  passa a pedir Rust 1.88 ou mais recente.
- **App:** o aviso do ecrã de criação do ID passa a dizer "Versão de testes. Até à versão 1.0, a app
  continua a ser melhorada." (nas seis línguas), em cinzento em vez de vermelho.
- **Documentação:** READMEs (português e inglês) reorganizados e mais curtos: o que a app faz, como
  protege a privacidade, estado, arquitetura, números e, no fim, os comandos para programadores.
  Versões atualizadas (Android 0.9.13). Removidos dois documentos antigos (`docs/final-report.md`,
  registo da fase de 13/10, e `docs/revisao-arquitetura.md`, pedido de revisão cancelado).
  Revistos o roadmap, o modelo de ameaças, as limitações conhecidas, os testes de campo e o README
  da app Android (estava desatualizado sobre notificações e testes feitos).
- **Site:** versão 0.9.12 no topo, e os textos "O que é a versão de testes" e "O aspeto da app"
  deixam de depender do número da versão (PT, EN e ES).

### 19 de outubro de 2026: melhorias dos testes da 0.9.11 (app 0.9.12, núcleo)

Nada muda no aspeto nem no que a app faz.

- **Chamadas:** se o Android der erro a ler o microfone, a app espera 10 ms antes de tentar de
  novo (antes repetia sem pausa e gastava 100% de um núcleo até desligar). Se o microfone morrer
  (outra app ficou com ele), deixa de o ler.
- **Lista de conversas (núcleo):** a consulta percorre os contactos aos saltos no índice em vez de
  ler uma entrada por mensagem. O tempo deixa de crescer com o histórico (medido no PC com 300 000
  mensagens: de ~33 ms para ~0,2 ms). Teste novo que compara com a consulta antiga
  (`storage/tests/messages_history.rs`) e o teste dos índices atualizado.
- **Fotos de perfil recebidas** maiores do que as da app (até 1024 px) são reduzidas ao abrir: até
  16 vezes menos memória, iguais no ecrã.
- **APK mais pequena:** só leva as 6 línguas da app (antes também as ~80 traduções das bibliotecas).

### 19 de outubro de 2026: novo design e foto de perfil corrigida (app 0.9.11, núcleo)

- **Novo design no Android.** Cabeçalho roxo com o fundo curvo em baixo, à direita, em todos os
  ecrãs. No ecrã inicial: o logo a rodar como um pião (só aqui; nos outros ecrãs fica parado),
  "Sealer", a tua foto com uma engrenagem e o teu ID abreviado (tocar mostra o QR). Lista com
  avatares maiores, hora em cima e, por baixo, as mensagens por ler ou uma seta. O botão "+"
  volta a ser um "+". Letra nova em toda a app: Plus Jakarta Sans (licença SIL OFL, incluída em
  `assets/licencas/`).
- **Ecrã Definições** em vez do menu de três pontos: foto de perfil, o meu ID, bloqueio da app,
  contactos bloqueados, "online", confirmações de leitura, notificações, idioma, versão e
  "Destruir perfil e dados". Abre-se na engrenagem do ecrã inicial.
- **Ecrã Novo contacto** em vez da janela: o teu código QR (Partilhar, Copiar), "Ler código QR",
  o ID e o nome da outra pessoa, e "Novo grupo".
- **Foto de perfil:** depois de escolher, a foto aparece já cortada com o aviso das 72 h e só
  fica com "Aplicar" (aparece "Foto atualizada"). Antes era aplicada logo, sem se ver nada, e quem
  tentava outra ficava preso nas 72 h. A original pode ter até 20 MB (antes 3 MB).
- **Correção (núcleo):** uma foto reduzida entre 9 974 e 10 000 bytes era recusada com "Essa
  imagem não serve como foto de perfil", porque com os 27 bytes do cabeçalho passava o limite de
  10 000 bytes de uma mensagem. O núcleo aceita agora até 9 973 bytes e a app reduz até 9 900.
  Teste novo em `ffi/tests/profile.rs`. Continua compatível com a 0.9.9 e a 0.9.10.
- iPhone: ainda sem o novo design (o código da app iPhone está no Mac).

### 19 de outubro de 2026: app em seis línguas (app 0.9.10)

- A app fala português, inglês, francês, espanhol, alemão e italiano. Segue a língua do telemóvel
  (português se não for nenhuma destas) e há um menu "Idioma" para escolher à mão. No Android 13+
  a escolha também aparece nas definições do sistema para a app.
- Todos os textos do ecrã passaram do código Kotlin para `res/values*/strings.xml` (cerca de 300).
  Os textos que vêm do núcleo Rust (foto, mensagem de voz, "atualiza a app") são traduzidos na app.
- A palavra para confirmar "Destruir perfil e dados" passa a ser a da língua (APAGAR, DELETE,
  SUPPRIMER, BORRAR, LÖSCHEN, ELIMINA).

### 19 de outubro de 2026: foto de perfil (app 0.9.9, núcleo)

- Menu "Foto de perfil": escolher uma foto da galeria (até 3 MB), ver e tirar. A app corta ao
  centro e reduz para um quadrado WebP de até 10 000 bytes; só se pode mudar de 72 em 72 horas
  (tirar pode-se sempre).
- A foto vai cifrada aos contactos guardados no tipo 12 do payload, dentro do Double Ratchet,
  num envelope do tamanho normal: o relay não sabe que é uma foto. Quem recebe também recusa
  fotos novas do mesmo contacto antes de 71 h.
- Aparece no lugar das iniciais em todo o lado: lista de conversas, conversa, grupos (foto de
  cada membro que seja contacto) e ecrã da chamada.
- Núcleo: `ffi/src/profile.rs` (`profile_photo_set`, `_remove`, `_sync`, `_mine`, `_of`,
  `_next_change_ms`), testes em `ffi/tests/profile.rs`. Ver `docs/protocol-spec.md`, secção 19.

### 19 de outubro de 2026: toque próprio da SEALER (app 0.9.8)

- As chamadas a chegar tocam um som próprio da SEALER (`res/raw/sealer_toque.ogg`, original,
  criado para a app) em vez do toque predefinido do telemóvel, com a app aberta ou fechada.
- Canal de notificação novo `chamadas_toque_v2` (o Android não deixa mudar o som de um canal já
  criado); o canal antigo `chamadas_toque` é apagado. As mensagens mantêm o som de aviso normal.

### 19 de outubro de 2026: ecrã da chamada sem dados técnicos (app 0.9.7)

- O ecrã da chamada deixa de mostrar a linha técnica (UDP ou WebSocket, ms ao servidor, pacotes
  perdidos e atrasados). Os números vão só para o registo, de 5 em 5 segundos
  (`adb logcat -s SealerRelay`).

### 19 de outubro de 2026: chamadas a tocar com a app fechada, fase 4b (app 0.9.6, relay)

- As chamadas tocam com a app fechada ou o telemóvel bloqueado: notificação de ecrã inteiro do
  Android, por cima do bloqueio, com Atender e Recusar e o toque do telemóvel. Atender abre só o
  ecrã da chamada por cima do bloqueio; quando a chamada acaba, a app fecha. Sem atender: "Chamada
  perdida" (sem o nome no ecrã bloqueado).
- A campainha (FCM) passa a abrir a base de dados em segundo plano: vai buscar o que chegou,
  decifra, envia os recibos e trata as chamadas. A notificação das mensagens continua genérica.
  Um só `QtClient` por processo (`SealerCore`, `Core.kt`), partilhado pelo ecrã e pelo segundo
  plano.
- A chave da base de dados e a camada exterior da lista de contactos passam para uma chave do
  Keystore que funciona com o telemóvel bloqueado (depois do primeiro desbloqueio); os ficheiros
  antigos passam ao formato novo da primeira vez que a app abre. Com o bloqueio da app ativo, nada
  abre sem o código.
- Com o bloqueio da app ativo: a oferta e o cancelamento vão com a campainha marcada como chamada
  (`wake`), o relay toca `"call"` / `"call_end"` e a app toca "Chamada SEALER" sem nome, pedindo o
  código ao atender. O relay e a Google ficam a saber o momento de uma chamada (aprovado).
- Relay: campo `wake` no `SendMessage`, limites próprios para as campainhas das chamadas.
- O ConnectionService do Android (auricular Bluetooth, carro, chamadas normais ao mesmo tempo)
  fica para um passo à parte.

### 19 de outubro de 2026: chamadas, jitter buffer mais tolerante (app 0.9.5)

- O jitter buffer espera até 200 ms por um pacote atrasado (antes 100 ms), contados no relógio e
  não em pedidos do altifalante. Um arranque lento do microfone do outro lado já não faz perder
  o início da chamada.
- Estatísticas separadas: perdidos na rede (reconstruídos pelo DRED) e atrasados (não chegaram a
  tempo). A linha técnica da chamada mostra os dois.
- Corrigido: se o microfone do outro telemóvel parasse mais de ~120 ms e perdesse essas
  amostras, todos os pacotes seguintes chegavam "atrasados" e esse lado ficava mudo até ao fim da
  chamada. Agora o jitter buffer recomeça no primeiro pacote que volta e corta o excesso de atraso
  de uma vez.
- App: o modo de chamada e o foco de áudio passam a ser pedidos antes de criar o microfone e o
  altifalante (mudar o modo depois reencaminhava o áudio a meio e o microfone parava cerca de meio
  segundo no início da chamada); o microfone tem 200 ms de folga em vez de 80.
- Testes novos: microfone lento no arranque, espera contada no relógio, microfone parado sem
  acumular (100 ms a 2 s) e rajada retida.

### 19 de outubro de 2026: chamadas de voz na app Android, fase 4a (app 0.9.4)

- Ligar a um contacto pelo botão de telefone na conversa; ecrã de chamada (a chamar, a tocar,
  a chegar, atendida com contador, terminada), atender, recusar, desligar, silenciar e
  altifalante. Só funciona com a app aberta (tocar com a app fechada é a fase 4b).
- Microfone com a fonte `VOICE_COMMUNICATION` (cancelamento de eco e de ruído do telemóvel),
  48 kHz mono; serviço em primeiro plano durante a chamada, para o microfone continuar com a app
  fora do ecrã; toque e vibração ao chegar, sinal de chamada ao ligar.
- Só tocam os contactos guardados: as chamadas de quem não está nos contactos são recusadas em
  silêncio.
- Núcleo: `CallAudio` (feature `calls` do `qt_ffi`, ligada na build Android) com o motor da
  fase 3; o motor entra na sala logo ao ligar ou quando toca, e as chaves do áudio chegam ao
  atender. A build Android passa a precisar do `cmake`.
- Teste de ponta a ponta como na app: `server/tests/call_audio_ffi.rs`.

### 19 de outubro de 2026: chamadas de voz, fase 3 (motor de áudio)

- Novo crate `call/` (`qt_call`): o motor das chamadas, que a app vai usar na fase 4.
  - `codec.rs`: Opus 1.6.1 (libopus oficial, pelo `opusic-sys`, com CMake) com DRED e PLC
    neural; voz em banda larga, frames de 20 ms, teto de 126 bytes, sem DTX.
  - `jitter.rs`: jitter buffer adaptativo (40 a 200 ms), reconstrução de perdas com o DRED do
    pacote seguinte, espera por pacotes atrasados e redução do atraso quando sobra.
  - `engine.rs`: microfone, Opus, cifra, relay (UDP com reserva por WebSocket), decifra, jitter
    buffer e altifalante, com estatísticas (ida e volta ao relay, perdas, DRED, PLC).
- `tools/call-demo`: ferramenta para ouvir no PC a própria voz depois de ir ao relay e voltar,
  com perdas simuladas (`--perda 15`).
- Testes: codec (DRED reconstrói três frames perdidos), jitter buffer e quatro testes de ponta a
  ponta pelo relay das chamadas (UDP, 15% de perdas recuperadas pelo DRED, WebSocket e passagem
  automática para WebSocket quando o UDP não responde).
- Compilar o workspace passa a precisar do `cmake` (para o libopus).

### 19 de outubro de 2026: chamadas de voz, fase 2 (relay das chamadas)

- Novo processo `qt-call-relay` (`server/src/call_relay.rs`): reencaminha os pacotes de áudio
  pela porta UDP 443, com salas anónimas (o pedido de entrada vai cifrado para a chave do relay,
  Noise_N, e não diz a conta). Reserva por WebSocket sem conta em `wss://<relay>/call` para redes
  que bloqueiam UDP. Mudança de IP a meio da chamada sem cortes. Limites por sala, por IP e por
  segundo. Não usa o Redis e não regista IPs.
- O código da sala passa a depender só da oferta: os dois telemóveis entram na sala enquanto
  toca (pré-aquecimento). As chaves do áudio continuam a usar os dois segredos.
- `deploy/aws/setup-ec2.sh` e `deploy/systemd/qt-call-relay.service`: instala o novo serviço e
  encaminha `/call` no HAProxy. Exige a porta 443 UDP aberta no Security Group (já aberta).
- Testes: `server/tests/call_relay_e2e.rs` (UDP, mudança de rede, um lado por UDP e outro por
  WebSocket, relay falso) e testes dos limites em `call_relay.rs`.

### 19 de outubro de 2026: chamadas de voz, fase 1 (núcleo Rust)

- Sinalização das chamadas 1 a 1 dentro do Double Ratchet (tipo 11): oferta, a tocar, resposta e
  desligar, com recusar, ocupado, sem resposta, cancelar, oferta atrasada e chamadas cruzadas
  (`ffi/src/call.rs`).
- Cifra dos pacotes de áudio no estilo WireGuard (`core/src/call.rs`): chaves por sentido
  derivadas dos dois segredos da chamada, janela anti-repetição, tamanho fixo, código da sala e
  etiqueta para o relay. Ver `docs/protocol-spec.md`, secção 18.
- Testes entre dois clientes, com os pacotes por UDP local (`ffi/tests/calls.rs`). Ainda sem
  relay de UDP, sem áudio e sem mudanças na app Android.

### 19 de outubro de 2026: documentação e site ao estado da 0.9.3

- Site: "Em versão de testes (0.9.3)", quadrado "O que é a versão 0.9.3" e "O que vem a seguir"
  (o que lá estava já existe; a seguir vem a Google Play e a App Store).
- READMEs (pt e en): estado da app 0.9.3 (grupos, voz, ficheiros, notificações push, segurança do
  protocolo), app iPhone 0.9.3 testada num iPhone, relay em `relay.sealer.pt` com HAProxy, tempos
  medidos a 19/10 e o script de instalação a partir do ramo `site-sealer`.
- `ROADMAP.md`, `android/README.md`, `ios/README.md`, `docs/known-limitations.md`, `site/README.md`
  e `CLAUDE.md`: notificações push e app iPhone já feitas; Caddy substituído por HAProxy.

### 19 de outubro de 2026: marcador do site com o tempo da mensagem típica

- O marcador ao lado do nome SEALER passa a dizer "entre telemóveis" (dizia "envio") e mostra a
  **mediana** do envio mais a mediana da entrega da última hora, em vez das médias. Umas poucas
  mensagens para telemóveis a dormir (acordados pela notificação, até 60 s) puxavam a média para
  mais de 1,5 s, com a mensagem típica a demorar menos de 200 ms.
- Relay: o `estado.json` passa a guardar também a última mediana conhecida
  (`ultima_mediana_ms`, `ultima_ida_mediana_ms`). O site volta à média se o relay ainda não as tiver.

### 19 de outubro de 2026: segurança do protocolo (app 0.9.3)

Quatro pontos da revisão de desenho, todos no núcleo Rust e no relay (o Kotlin não mudou):

- **PQXDH v2:** a derivação do segredo inicial passa a seguir a especificação PQXDH do Signal
  e inclui a transcrição (todas as chaves públicas e o ciphertext ML-KEM). As sessões que já
  existem continuam a funcionar; as novas usam o v2. `core/src/pqxdh.rs`.
- **Chaves que rodam e chaves de uso único:** bundle v4 com número e data da signed pre-key
  (assinados); a signed pre-key e a prekey ML-KEM rodam a cada 7 dias (as anteriores ficam
  ~35 dias); bundles com mais de 90 dias são recusados. O relay guarda um lote diário de 100
  one-time prekeys por conta e entrega uma (e apaga-a) por pedido de bundle.
  `ffi/src/prekeys.rs`, `core/src/identity.rs`, `server/src/store.rs`.
- **Enchimento:** envelope v3; o texto é arredondado a 256, 1024, 4096 ou 10 240 bytes antes de
  ser cifrado, para o relay não distinguir recibos, presença e mensagens curtas.
- **Contas em massa:** prova de trabalho Argon2id (8 MiB, 7 bits, uns segundos num telemóvel,
  uma vez por conta) exigida pelo relay a contas novas; limites por conta de 300 envelopes e
  60 pedidos de bundle por minuto. Redis: comando `LPOP` acrescentado ao ACL.
- **Compatibilidade:** o relay aceita apps antigas e novas, mas as apps 0.9.3 e 0.9.2 não
  conversam entre si (envelope e bundle novos). Atualizar o relay primeiro e depois todos os
  telemóveis.

### 19 de outubro de 2026: fim do sslip.io, documentação e um aviso do compilador

- O site (`site/wrangler.jsonc`) e a configuração da app iPhone passam a usar `relay.sealer.pt`. O
  servidor deixa de servir `13-51-46-213.sslip.io` (as apps Android de teste já têm a 0.9.2).
- `server/src/blobs.rs`: deixa de usar `fetch_update`, que o Rust novo dá como antiquado (era o único
  aviso na compilação do relay). O comportamento é igual.
- Documentação: o HAProxy no lugar do Caddy (diagramas, serviço e exemplo de configuração), as
  medições com o HAProxy em `docs/performance.md`, e o pedido de revisão atualizado.

### 19 de outubro de 2026: relay em relay.sealer.pt (app 0.9.2)

- As apps passam a usar **`wss://relay.sealer.pt`** em vez de `13-51-46-213.sslip.io` (um serviço de
  terceiros). O nome é um registo DNS do `sealer.pt`, sem proxy: o tráfego continua a ir direto do
  telemóvel para o servidor, e mudar de servidor passa a ser só mudar o DNS. O pin não muda.
- `deploy/aws/setup-ec2.sh` aceita vários nomes (`setup-ec2.sh relay.sealer.pt 13-51-46-213.sslip.io`):
  um certificado Let's Encrypt para cada um, o HAProxy serve-os todos e os nomes ficam em
  `/etc/qt/domains` para as atualizações seguintes. Acrescentar um nome não corta as ligações abertas.
- App Android **0.9.2** (`versionCode` 9): só muda o endereço do relay.

### 19 de outubro de 2026: limites contra abusos no relay

- **Por IP, no HAProxy** (`deploy/aws/setup-ec2.sh`): no máximo 100 ligações abertas ao mesmo
  tempo e 100 ligações novas a cada 10 s por IP; o pedido inicial tem de chegar em 5 s (antes
  10 s). As ligações recusadas contam-se no HAProxy (`dcon`, por `stats socket`).
- **No relay** (`server/src/limits.rs`): no máximo 2 000 ligações por autenticar ao mesmo tempo
  (`RELAY_MAX_UNAUTHENTICATED`), para uma enxurrada de ligações caladas não tirar a vaga a quem
  já está autenticado; no máximo 8 ligações por conta (`RELAY_MAX_CONNECTIONS_PER_USER`): acima
  disso fecha-se a mais antiga dessa conta, por isso um telemóvel que muda de rede nunca fica de
  fora por causa das ligações mortas que deixou. Handshake e autenticação em 5 + 5 s (antes
  10 + 10). Novos contadores na linha de estatísticas: `por_autenticar_cheio` e
  `antigas_fechadas`. Teste: `server/tests/relay_limits.rs`. As apps não mudam.
- `RELAY_MAX_CONNECTIONS` por omissão no `setup-ec2.sh` passa a 10 000.

### 18 de outubro de 2026: site, tempo entre telemóveis

- O marcador de estado do sealer.pt mostra o tempo de telemóvel a telemóvel (envio até ao
  servidor mais entrega até o destinatário confirmar), em vez de só o envio. `site/src/status.js`.

### 18 de outubro de 2026: HAProxy em vez do Caddy no relay

- **Teste de carga na AWS** (t3.small, `conntest` com TLS real): 4 000 e 6 000 ligações sem
  falhas; perto das 8 000 a memória acabava. Medido no servidor: o Caddy gastava ~137 KB por
  ligação, quase toda a memória. Detalhes em `docs/performance.md`, secção "Na AWS".
- `deploy/aws/setup-ec2.sh`: TLS pelo **HAProxy** com certificado Let's Encrypt do **certbot**
  (renovação automática, o HAProxy recarrega sem cortar ligações). Sem registos de acesso, como
  antes. O `/estado.json` do site passa a ser servido por um serviço mínimo só em localhost
  (`qt-estado`). O script desliga e remove o Caddy. `RELAY_MAX_CONNECTIONS` passa a 16 000.
  A app, o endereço e o pin não mudam.

### 18 de outubro de 2026: notificações só para mensagens, contactos apagados e scroll (app 0.9.0)

- **Notificações só quando há mensagens.** O relay tocava a campainha (FCM) para tudo o que
  chegava a uma app fechada, incluindo a presença (enviada a todos os contactos sempre que alguém
  abre a app) e os recibos. Agora recibos, reações, convites, presença e envelopes de grupo que
  não são mensagens vão com `SendMessage{…, silent: true}` e não tocam (`OutboxItem.silent`,
  `relay_request_send_item`). Relay e app têm de ser atualizados; um relay antigo ignora o campo.
- **Apagar um contacto já não o impede de voltar a escrever.** Antes, só quem apagava esquecia a
  sessão; o outro continuava a cifrar com ela e as mensagens perdiam-se em silêncio. Agora, antes
  de apagar as chaves, a app envia o aviso cifrado "sessão terminada" (tipo 10,
  `delete_contact_notify`; também ao recusar um convite, nunca a quem está bloqueado). Quem o
  recebe apaga a sessão (fica com o contacto e o histórico), abre logo uma nova e volta a enviar o
  que não tinha sido entregue; do outro lado aparece como convite. Só vale para contactos apagados
  a partir desta versão: para os que já estavam avariados, quem não consegue escrever apaga o
  contacto e volta a adicioná-lo.
- **Scroll automático com muitas mensagens seguidas** (a dois e nos grupos, `FollowNewest` em
  `MainActivity.kt`): a lista só deixa de seguir as mensagens novas quando a pessoa a arrasta para
  cima. Antes, quando chegavam várias de uma vez, ficava parada.
- Testes: `server/tests/relay_push_send.rs` (envio `silent` não toca) e
  `ffi/tests/session_reset.rs` (aviso, reenvio do que se perdeu, convite, bloqueado não avisa).

### 17 de outubro de 2026: ficheiros e anexos nos grupos (app 0.9.0)

- **Ficheiros até 25 MB** (PDF, documentos, folhas de cálculo, etc.): botão "+" > "Ficheiro".
  Vão tal como estão, cifrados ponta a ponta como as imagens, com o nome e o tipo dentro da
  mensagem cifrada (corpo do anexo na versão 2; imagens e voz continuam na versão 1). Ao tocar:
  "Abrir" (a outra app lê o ficheiro da memória do SEALER, sem cópia decifrada em disco, ver
  `SealerFileProvider` em `android/.../Files.kt`) ou "Guardar" (onde o utilizador escolher).
- **Fotos, GIFs, voz e ficheiros nos grupos.** O ficheiro cifrado vai uma vez para o relay e a
  chave vai a cada membro (`group_attachment_uploaded`). Nos grupos, quem recebe não apaga o
  ficheiro do relay (os outros ainda o podem estar a descarregar): o relay apaga-o ao fim de
  24 horas. O relay não mudou.
- A caixa de escrita é a mesma nas conversas a dois e nos grupos (`Composer.kt`). Nos grupos,
  tocar numa imagem abre-a em ecrã inteiro.
- Apagar um grupo apaga também os anexos cifrados dele no telemóvel (antes ficavam no disco).
- Testes: `ffi/tests/files.rs` (ficheiro a dois, nomes e tipos inválidos, foto, voz e ficheiro
  num grupo com três pessoas, quem saiu do grupo não envia).
- O exemplo `server/examples/peer.rs` voltou a compilar.

### 17 de outubro de 2026: notificações com a app fechada (FCM)

- **Relay** (`server/src/push.rs`): pedido `RegisterPushToken` (o próprio utilizador, token
  validado, vazio apaga), guardado no Redis 90 dias e apagado com a conta. Quando chega uma
  mensagem e ninguém está à espera, pede à Google (FCM HTTP v1) para acordar o telemóvel, com
  uma mensagem vazia; no máximo uma vez a cada 10 s por pessoa. Tokens mortos são apagados.
  Sem `QT_FCM_CREDENTIALS`, desligado. Deploy: `/etc/qt/fcm.json` (ver `setup-ec2.sh`).
- **App** (`Push.kt`): Firebase Cloud Messaging iniciado à mão (sem Analytics, sem início
  automático, sem o plugin google-services), aviso "Tens mensagens novas" sem decifrar nada,
  permissão de notificações no Android 13+ (pedida uma vez), interruptor "Notificações" no
  menu. O interruptor antigo com esse nome era o dos recibos de leitura: passa a chamar-se
  "Confirmações de leitura".
- Política de privacidade e `docs/google-play.md` atualizados (a Google como entregadora do aviso).

### 17 de outubro de 2026: identificador da app `sealer.app`

- O identificador da app (Play Store) passa de `pt.qt.app` para `sealer.app`. Para o Android é
  uma app nova: a instalação antiga fica ao lado, com as suas conversas e o seu ID; a nova
  começa do zero (ID novo, trocar os QR outra vez). O código Kotlin continua em `pt.qt.app`.

### 17 de outubro de 2026: revisão de QA da app Android (0.9.0)

- **Memória:** os anexos decifrados ficam numa cache em memória até 40 MB, partilhada pela
  conversa e pelo ecrã inteiro (antes eram decifrados de novo a cada scroll). Imagens e GIFs
  maiores que 8 MB mostram "Toca para ver" em vez de abrirem sozinhos. A cache é posta a
  zeros ao bloquear a app e ao apagar conversas, contactos ou grupos.
- **Gravações de voz** que ficaram de uma vez em que a app morreu a meio são apagadas no arranque.
- **Conversas longas:** cada mudança relê no máximo as 200 mensagens mais recentes (antes,
  todas as que já estavam abertas); recibos e reações em mensagens mais antigas são relidos
  uma a uma.
- **Tapjacking:** os diálogos de apagar, bloquear, destruir o perfil e mudar o código ignoram
  toques que chegam através de outra janela por cima.
- **Registos:** a release já não tem `Log.v/d/i` (o R8 apaga-os); ficam os avisos de ligação.
- **Testes e build automática:** testes unitários Kotlin (`android/app/src/test`) e o workflow
  `.github/workflows/android.yml` (compila, testes, R8 da release e Android Lint a cada push).

### 17 de outubro de 2026: grupos, reações, voz, convites e online (app 0.8.0)

- **Online / offline** (`ffi/src/presence.rs`): ponto verde nos contactos com a app aberta,
  "online" na conversa e a fila "Online agora" no topo da lista. Só para contactos e cifrado
  (tipo 9); nada fica guardado. Interruptor "Mostrar quando estou online" no menu: desligado,
  não mostra o meu estado nem vejo o dos outros.

- **Grupos até 10 pessoas, com até 3 administradores** (`ffi/src/groups.rs`). Envio a cada
  membro pelas sessões 1 a 1: o relay não muda e não sabe que existe um grupo. Os
  administradores mudam o nome, acrescentam e retiram pessoas e escolhem os outros
  administradores; cada mudança sobe a versão do grupo. Quem sai avisa os outros. Para
  criar não é preciso escolher pessoas nem dar nome (sem nome fica "Grupo"). Nesta
  versão: texto e reações (ver `docs/protocol-spec.md`, secção 15).
- **Reações com emoji** a mensagens, imagens e mensagens de voz (toque longo). Cifradas como
  as mensagens; tocar outra vez troca, "Retirar a minha" apaga.
- **Mensagens de voz até 10 segundos** (botão do microfone com a caixa vazia). AAC, cifradas
  ponta a ponta como as imagens; ouvem-se a partir da memória, sem ficheiro decifrado em disco.
- **Convite:** quem escreve a alguém pela primeira vez manda, cifrado, o seu Public ID. Do
  outro lado aparece "Aceitar / Recusar / Bloquear", sem pedir o ID à mão.
- Sessões cruzadas: quem ganha o desempate passa a ler a mensagem do outro em vez de a
  descartar, e os envelopes de grupo por enviar são cifrados de novo com a sessão nova.
- iPhone: ainda não tem estas funções (ver `docs/known-limitations.md`, ponto 38).

### 17 de outubro de 2026: bloquear contactos e termos de utilização

- **Bloquear** (núcleo Rust, `block_contact` / `unblock_contact` / `blocked_contacts`): as
  mensagens de quem está bloqueado são descartadas ao chegar (a sessão avança, mas nada é
  guardado nem mostrado e não se enviam recibos), e nada lhe pode ser enviado (o bloqueio
  está em `seal_with`, por onde passa tudo o que sai). O que estava por enviar sai da
  outbox. Fica só no telemóvel, na base cifrada; o relay não sabe de nada.
- Android: "Bloquear" e "Desbloquear" no menu da conversa, aviso no lugar da caixa de
  escrita, "Bloqueado" na lista de conversas e "Contactos bloqueados" no menu principal.
- **Termos de utilização** em sealer.pt/termos, e a app pede que sejam aceites (com a
  política de privacidade) antes de a usar. Os dois pontos são exigidos pela política de
  conteúdo gerado por utilizadores da Google Play.
- Android: sem avisos de ligação na app ("A ligar ao relay", "Sem ligação" e o diagnóstico
  da entrega imediata). Os problemas de ligação ficam só no registo (`adb logcat -s SealerRelay`).

### 17 de outubro de 2026: apagar a conta também no servidor (app 0.7.0)

- Novo pedido `DeleteAccount`: o relay apaga o bundle de chaves públicas, a fila de
  mensagens e a contagem de anexos da identidade. Durante 30 dias recusa mensagens
  para ela (quem envia vê "apagou a conta").
- O bundle de chaves públicas expira ao fim de 90 dias (antes nunca expirava). As
  apps voltam a publicá-lo sempre que abrem (arranque ou desbloqueio).
- Android: "Apagar todos os dados" passa a "Destruir perfil e dados" e apaga
  primeiro no servidor. Sem rede, não apaga nada e pergunta se quer tentar de novo
  ou apagar só do telemóvel. Requisito da Google Play (eliminação de conta).
- Android, bateria: 30 s depois de a app sair do ecrã, fecha as ligações ao relay e
  para a receção (antes ficavam até 3 ligações abertas, cada uma a acordar o rádio de
  25 em 25 s, sem mostrar nada ao utilizador). Ao voltar, religa e vai buscar o que
  chegou. Anexos a meio terminam primeiro (até 5 min). Uma ligação fechada já não se
  reabre sozinha por causa de um pedido que estivesse a meio.
- Android, memória: a sessão Noise (no Rust) de cada ligação é libertada logo que a
  ligação fecha ou falha, em vez de esperar pelo GC do Java.
- Android, rede: um só cliente HTTP (OkHttp) para as 3 ligações, em vez de um por
  ligação. Os certificados do sistema são preparados uma vez e as ligações novas
  reaproveitam a sessão TLS: religar ao voltar à app gasta menos tempo e bateria.
- Android, ao ligar: vai buscar as mensagens primeiro e só depois republica as chaves
  públicas (pela ligação dos anexos, para não passar à frente). Ao voltar de segundo
  plano já não as republica (duram 90 dias e não mudaram).

### 17 de outubro de 2026: tempo de envio

- **Tempo médio de envio** (telemóvel de quem envia até ao relay): logo depois de
  aceitar uma mensagem, o relay manda um ping nessa ligação e mede a ida e volta
  até ao pong; o envio é metade. Não precisa de apps novas (os clientes WebSocket
  respondem sozinhos aos pings). Fica em `estado.json` (`ida_media_ms`,
  `ida_mediana_ms`, `ida_p95_ms`, `ida_amostras`, `ultima_ida_ms`) e é o número
  que o site sealer.pt mostra ("envio 23 ms"), em vez do tempo de entrega.

### 16 de outubro de 2026: entrega imediata e tempo de entrega

- **Entrega imediata** (`WaitMessages`): as apps deixam de perguntar ao relay de
  3 em 3 s e ficam à espera numa ligação própria; o relay responde no instante
  em que a mensagem chega. No relay, o tempo até à entrega passou de ~1 s em
  média para menos de 1 ms (`docs/performance.md`). Compatível nos dois
  sentidos: apps antigas continuam a perguntar; com um relay antigo, as apps
  novas voltam a perguntar de 3 em 3 s.
- As apps mostram a mensagem recebida logo que está guardada no telemóvel, antes
  de a confirmarem ao relay (antes esperavam por mais duas idas à rede, ~110 ms
  com o relay na Suécia). A confirmação continua a ser feita só depois de guardar.
- **Tempo médio de entrega** medido pelo relay (só em memória, sem remetente,
  destinatário nem tamanho), publicado em `/estado.json` e mostrado em sealer.pt.
- Site sealer.pt: política de cookies, página "Como funciona" com animação e
  marcador de estado do serviço.

### Outubro de 2026: apps, relay na AWS, imagens

- **Apps Android e iPhone** (o nome que aparece no telemóvel é SEALER), com o
  mesmo núcleo Rust: lista de conversas, chat, histórico persistente no SQLCipher, estados
  *enviada / entregue / vista* por recibos cifrados, fila de saída que
  sobrevive a reinícios, apagar conversa e apagar contacto.
- Sessões cruzadas (as duas pessoas a escreverem primeiro ao mesmo tempo):
  desempate determinístico e reenvio, com testes.
- **Relay na AWS** (`deploy/aws/setup-ec2.sh`): Redis só em localhost com
  ACL, relay como serviço systemd isolado, Caddy com TLS automático. Build
  *release* das apps com o relay fixo e pinning obrigatório.
- `messenger` (terminal) passa a usar o mesmo núcleo das apps e fala com
  elas pelo relay.
- **Bloqueio da app no Android**: código próprio (Argon2id) e desbloqueio com
  biometria.
- Marca SEALER: logotipo, nome, tema roxo e logotipo a rodar.
- **Adicionar contactos por código QR**, nas duas apps.
- **Imagens e GIFs até 25 MB**, cifrados ponta a ponta com uma chave por
  ficheiro, guardados cifrados no disco do relay no máximo 24 horas
  (`docs/protocol-spec.md`, secção 14).
- Apps: a opção do menu "Recibos de leitura" passa a chamar-se
  "Notificações" (continua a decidir só se a app envia o aviso de "vista").
- Relay: respostas sem Nagle (`TCP_NODELAY`); guardar uma mensagem com 2
  idas ao Redis em vez de 4; confirmar mensagens sem ler a fila inteira.
  Medições em `docs/performance.md`.
- Redis: comandos dos anexos acrescentados ao utilizador `qt-relay` (sem eles
  o envio de imagens dava "erro interno" no servidor), com um teste que usa
  a lista de comandos de produção (`server/tests/redis_acl.rs`).
- Testes de campo com cinco aparelhos em três plataformas
  (`docs/field-tests.md`) e medições com 10 000 ligações em simultâneo.

### Segurança (hardening)
- **Quebra de formato:** bundle v3 (assinatura cobre a identidade X25519),
  envelope v2 (cabeçalho do ratchet com `PN`, AD de sessão no AAD, caixa
  selada ligada ao envelope). Sessões/bundles v2 existentes deixam de ser
  compatíveis; estados de ratchet antigos em disco ainda carregam.
- Safety numbers (`QtClient::safety_number`, ecrã na app Android).
- Relay valida que o bundle registado pertence à identidade autenticada
  (chave Ed25519 + assinatura): ninguém escreve no bundle de outrem.
- Remetente autenticado pela sessão (AD com as duas identidades), não pelo envelope.
- Ratchet: `PN`, limite global de chaves puladas (2000), `decrypt` transacional
  sobre cópia, rejeição explícita de mensagens repetidas/antigas.
- Transporte: o cliente Rust só aceita `wss://` (ou `ws://` em loopback);
  o relay escuta por omissão em `127.0.0.1` atrás de um proxy TLS
  (`docs/deploy-wss.md`). Android: release só `wss://`, `usesCleartextTraffic=false`,
  assinatura com keystore própria (o build falha sem ela).
- Legado: a CLI (`cli/`, X3DH clássico) já não consegue registar bundles num relay
  atualizado (formato v3 obrigatório).

### Adicionado
- Modelo de ameaças completo (`docs/threat-model.md`)
- Roadmap dedicado (`ROADMAP.md`)
- Este changelog
- Testes por propriedades do Double Ratchet (`core/tests/proptest_ratchet.rs`)
  e testes com entradas inválidas e forjadas (ver `docs/threat-model.md`, §3)

### Corrigido
- `SECURITY.md` já não tem placeholders por preencher: usa o mecanismo
  de report privado do GitHub em vez de um email/PGP que nunca existiu
- `cli/src/bin/messenger.rs` entrava em pânico ao processar uma mensagem
  vazia vinda do servidor (`bytes[0]` sem verificação de tamanho). O
  problema desapareceu quando o `messenger` foi reescrito sobre o núcleo
  das apps (commit `84c0b62`).

> **Correção a este registo (outubro de 2026):** uma versão anterior deste
> ficheiro dizia que existiam testes `core/tests/fuzz_lite.rs` e
> `cli/tests/fuzz_lite.rs`, alvos de `cargo-fuzz` em `core/fuzz/` e uma
> correção do `bytes[0]` na CLI antiga. Esses ficheiros e essa correção
> nunca chegaram a este repositório (o commit `7daee18` só alterou
> documentação). O fuzzing guiado por cobertura continua por fazer
> (`ROADMAP.md`).

## Núcleo e infraestrutura (fundação técnica)

Por ordem de construção, não de data exata (este projeto começou como
uma conversa de arquitetura antes de existir código):

- **`core/`**: X3DH clássico, depois assinatura Ed25519 da signed
  pre-key, one-time pre-keys, PQXDH (X25519 + ML-KEM/Kyber), Double
  Ratchet, e por fim Sealed Sender, cada peça com testes automáticos
  dedicados, incluindo casos que devem falhar (assinatura adulterada,
  mensagem adulterada, chave errada)
- **`transport/`**: Noise Protocol Framework (`Noise_XX_25519_ChaChaPoly_SHA256`)
  sobre WebSocket, testado com uma ligação real em `localhost`
- **`server/`**: relay com fila de mensagens e diretório de pre-keys,
  primeiro em memória, depois migrado para Redis real, com TTL
  automático; testado sobrevivendo a `kill -9` do processo
- **`storage/`**: armazenamento local em SQLCipher (identidade e
  sessões), testado com passphrase errada a falhar e o ficheiro em disco
  confirmado como genuinamente cifrado (sem a assinatura padrão do SQLite)
- **`cli/`**: primeiro uma demo num único processo, depois um cliente
  persistente a sério (`identity`/`register`/`send`/`receive`), testado
  com processos completamente separados a comunicar entre si
- **`ffi/`**: bindings uniffi para Kotlin, Swift e Python, testados via
  Python primeiro, depois com um `NoiseSession` com estado para expor
  também a camada de transporte, e finalmente testado num Android real
  (emulador)
- **Persistência da chave estática Noise do servidor** entre reinícios
  (antes gerada de novo a cada arranque)

## Organização e confiança

- Projeto renomeado de "secure-messenger" para **QT**
- Licença AGPL-3.0 completa (antes tinha um placeholder por preencher)
- README reorganizado com moldura honesta de "protótipo técnico", não
  "produto acabado", logo na primeira frase
- Versão em inglês do README (`README.en.md`)
- `.gitignore` criado (faltava desde o início)
- CI (GitHub Actions) configurado: testes do workspace inteiro, feature
  pós-quântica, bindings FFI via Python, `cargo clippy` estrito,
  `cargo audit`
