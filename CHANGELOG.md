# Changelog

Formato inspirado em [Keep a Changelog](https://keepachangelog.com/).
Datas aproximadas: este projeto ainda não segue versionamento semântico
formal (não há releases numeradas ainda, ver `ROADMAP.md`).

## [Não lançado]

### 31 de outubro de 2026: nomes e ligações nas chamadas de grupo (Android e iPhone 0.9.67)

- Membros do grupo que não são contactos um do outro (e nunca escreveram no grupo) não tinham
  sessão cifrada entre si: a perna deles nunca se criava e não se viam nem se ouviam. Antes de
  ligar ou entrar, a app cria as sessões que faltam (como antes de enviar uma mensagem ao grupo).
  No núcleo, quem já está na chamada repete o "entrei" a quem chega quando é o outro que tem de
  oferecer (o primeiro "entrei" pode não ter chegado por falta de sessão). Teste novo.
- Nome nesta chamada: antes de ligar ou entrar escolhe-se um nome (fica guardado para a próxima).
  Vai cifrado a cada participante (op 8, `group_call_set_name`); quem não me tem nos contactos vê
  esse nome, quem tem vê o nome que me deu. Nunca o QT ID.
- O aviso "Não foi possível usar a câmara" nunca aparece (chamadas a dois e de grupo): os erros da
  câmara só vão para o registo.

### 30 de outubro de 2026: a minha câmara em cima nas chamadas de grupo (Android e iPhone 0.9.66)

- O quadrado grande em cima passa a ser sempre o meu (a minha câmara, ou a minha foto) e os outros
  ficam na grelha de 2, sempre pela mesma ordem. Antes ia para cima quem falava, e cada troca
  refazia os vídeos (flashes constantes). Quem fala continua com a moldura na cor da app.

### 30 de outubro de 2026: correções das chamadas de grupo (Android e iPhone 0.9.65)

- Android: o aviso "Não foi possível usar a câmara" deixava de parar. As sessões da câmara
  refeitas a meio (uma pausa curta, alguém a entrar) falhavam e cada falha voltava a avisar. Agora
  uma sessão substituída não conta como erro, o aviso aparece no máximo uma vez, e o codificador
  só pára ao fim de 3 s sem nada a enviar.
- Android: o vídeo de alguém ficava preto quando passava para o quadrado grande (ou saía dele):
  o quadrado antigo apagava o ecrã do novo. Agora só o quadrado atual apaga o seu ecrã.
- O QT ID nunca aparece na chamada de grupo: quem não está nos contactos aparece como "Membro".
- Quem recusa (ou não atende) uma chamada de grupo pode entrar depois, enquanto houver alguém:
  "Entrar na chamada" no topo da conversa do grupo (`group_call_ongoing`, `group_call_rejoin`,
  evento `Ongoing`). Teste novo: recusar e entrar depois, com as duas pernas ligadas.
- Diagnóstico da chamada de grupo, como nas chamadas a dois: premir o tempo da chamada abre os
  números de cada participante (com o nome, sem IPs nem ids), com "Copiar" (`diagnostic_of`).

### 30 de outubro de 2026: chamadas de grupo no iPhone (0.9.64, build 64)

- O mesmo que no Android 0.9.63: botões na conversa do grupo, o aviso do IP com "Aceitar" antes
  da primeira chamada de grupo, o ecrã (`GroupCallScreen.swift`: quem fala em cima, os outros e eu
  na grelha de 2, os botões a desaparecer com vídeo) e o `GroupCallModel` (`Calls/GroupCall.swift`).
- `CallAudioIO`, `VideoSender` e `VideoReceiver` passam a usar protocolos (`AudioPipe`,
  `VideoOut`, `VideoIn`), que o `CallAudio` e o `GroupAudio` cumprem.
- Sem CallKit por agora: a chamada de grupo toca com a app aberta (toque da app em ciclo) e a
  sessão de áudio é ativada pela app. Durante a chamada de grupo as ligações ao relay não pausam.
- Textos novos gerados com `scripts/strings-ios.py` (as mesmas chaves do Android, nas 10 línguas).

### 30 de outubro de 2026: chamadas de grupo no Android (0.9.63, build 63)

- Botões de chamada e videochamada na conversa do grupo. Antes da primeira chamada de grupo
  aparece "Nas chamadas de grupo, o teu endereço de rede fica visível aos outros participantes"
  com "Aceitar" (sem isso não se liga nem se entra).
- Ecrã novo (`GroupCallScreen.kt`, layout do esboço com as cores da app): quem está a falar num
  quadrado grande em cima (moldura com o gradiente da app), os outros e eu numa grelha de 2, com o
  nome e o ícone do microfone ou da câmara. Com vídeo, o topo e os botões desaparecem ao fim de
  3,5 s sem toques, como na videochamada a dois.
- `GroupCallController` (`GroupCall.kt`): toque, entrar, sair, microfone e altifalante (a mistura
  vem do Rust), uma só codificação da câmara para todos, um descodificador por participante com a
  câmara ligada, serviço da chamada e pausa da câmara fora do ecrã.
- `CallAudioIO`, `VideoSender` e `VideoReceiver` passam a usar interfaces (`AudioPipe`,
  `VideoOut`, `VideoIn`), para servirem a chamada a dois e a de grupo.
- Ainda não: a chamada de grupo não toca com a app fechada (só com a app aberta ou em segundo
  plano), nem passa pelo sistema de chamadas do Android (Bluetooth do carro).

### 30 de outubro de 2026: núcleo das chamadas de grupo (0.9.62)

- Chamadas de grupo até 5 pessoas (`ffi/src/group_call.rs`, tipo 14 no Double Ratchet): uma malha
  de chamadas a dois ("pernas"), cada uma com chaves e motor próprios; o relay não muda. Quem
  oferece cada perna é o participante com o id menor (sem corridas, seja qual for a ordem de
  entrada). Só entre membros do grupo, nunca com bloqueados; o 6.º não entra.
- Voz: ligação direta com todos os participantes (mesmo os que não são contactos) e, sem direto,
  pelo relay. Vídeo: só pela ligação direta. Entrar numa videochamada de grupo é aceitar ver.
- `GroupAudio`: o microfone vai para todas as pernas, o altifalante recebe a mistura, quem fala
  mais alto passa a ser "quem está a falar" (fica pelo menos 1,5 s). Vídeo: uma só codificação
  para todos, ao débito da perna mais fraca; 360p por omissão, 720p no Wi-Fi (480p em dados) a
  falar (1,5 s para subir, 4 s para descer).
- Uma chamada de cada vez: convite de grupo durante uma chamada a dois (ou o contrário) dá
  ocupado.
- Testes com telemóveis reais (QtClient + motores, só pelo direto): 3 pessoas ouvem-se e sabem
  quem fala, vídeo de um chega aos outros dois, 5 entram e o 6.º não, o último a sair pára o toque
  de quem ainda tocava, sem aceitar o aviso não se liga, quem não é do grupo não liga.

### 30 de outubro de 2026: qualidade do vídeo e voz à frente (Android e iPhone 0.9.61, build 61)

- **Teto do vídeo:** com dados móveis (de qualquer dos lados) continua 720p; com Wi-Fi dos dois
  lados deixa de haver teto fixo: sobe até ao que a ligação deixa, sem passar do que o codificador
  (e a câmara) de quem envia faz nem do que o descodificador de quem recebe aguenta. A escada ganha
  1440p (até 8 Mbit/s) e 2160p (até 16 Mbit/s). Cada app diz o que faz (`video_set_encode_limit`,
  `video_set_decode_limit`) e quem recebe envia a sua capacidade pelo direto a cada 2 s (pacote de
  vídeo novo `[3][2 altura]`; as apps antigas deitam-no fora). Sem ela (app antiga do outro lado),
  fica nos 1080p de antes.
- **Arranque conforme a rede:** 1,2 Mbit/s (720p) com Wi-Fi dos dois lados; 500 kbit/s (360p) com
  dados móveis (na 0.9.53 arrancava sempre em 1,2 Mbit/s e entupia o envio pelos dados).
- **A voz manda:** o atraso a mais do áudio que chega pela ligação direta (fila parada contra a
  menor chegada dos últimos 10 s, ou a variação acima de 40 ms) vai no "atraso de fila" dos
  relatórios do vídeo (o mesmo campo: as apps antigas também reagem). O vídeo desce antes de a voz
  chegar atrasada.
- **Jitter buffer:** o atraso a mais desce primeiro nos silêncios (um frame baixinho, já
  descodificado, salta-se sem se ouvir) e, com fala contínua, mais depressa quanto maior o excesso
  (600 ms a mais saem em ~3 s; antes ~15 s).
- **Bateria (Android):** sem nada a enviar (pausa, sem ligação direta, o outro ainda não aceitou)
  o codificador H.264 para e a câmara só alimenta a pré-visualização (antes codificava tudo e o
  Rust deitava fora). No iPhone já não codificava.
- Imagem completa maior aceite: 2 MB (era 1 MB), para o 4K. Nível H.264 5/5.1 acima de 1080p
  (Android); 4K da câmara reduzido a 1440p quando é esse o pedido (iPhone).
- Testes novos: escada até 2160p com Wi-Fi e aparelhos 4K, teto pelo descodificador do outro,
  1080p sem saber dos aparelhos, 360p/720p com dados, voz atrasada baixa o vídeo, capacidade ida e
  volta, atraso grande do jitter buffer sai depressa, salto nos silêncios, medidor do atraso do
  áudio, e ponta a ponta: Wi-Fi com aparelhos 4K sobe acima de 1080p sem nada no relay.

### 30 de outubro de 2026: vídeo só pela ligação direta, sem fichas, sem relay de teste (Android e iPhone 0.9.53)

- O vídeo das chamadas vai **só pela ligação direta** entre contactos mútuos e nunca pelo relay
  (nem imagens, nem relatórios, nem pedidos de imagem completa; na duplicação vai só pelo direto).
  Sem direto (contacto não mútuo ou bloqueado, rede que não deixa, caminho que caiu) a chamada
  continua só com voz pelo relay e o vídeo fica em pausa, sem nada no ecrã; volta sozinho com o
  direto, com uma imagem completa. A voz e a regra do direto (`p2p_allowed`) não mudam.
- Saem as **fichas anónimas** do vídeo (0006): `core/src/fichas.rs`, `call/src/fichas.rs`,
  `ffi/src/fichas.rs`, `server/src/fichas.rs`, a crate `voprf` (e a `elliptic-curve` e a
  `crypto-bigint`, que só vinham com ela; sai o aviso RUSTSEC ignorado no `.cargo/audit.toml`) e os
  testes delas. Saem também os tetos do vídeo pelo relay (360p e 240p): fica o do direto (1080p no
  Wi-Fi sem limite de dados, 720p no resto).
- Relay das mensagens: sem emissão de fichas, sem `AdminSetFichasQuota`, sem a semente e sem as
  chaves `{p}:fichas:*`. `IssueFichas` (das apps antigas) responde sempre `FICHAS_OFF`.
- Relay das chamadas: só voz. Deita fora o vídeo das apps antigas e responde "sem vídeo pelo
  relay" (modo 2, no máximo de 5 em 5 s) para elas pararem de enviar; os pacotes de resgate e da
  chave do mês ficam sem resposta. Sem Redis (sai o utilizador `qt-fichas` da ACL), sem a semente
  e sem a conta do mês em disco. Saem `CALL_RELAY_VIDEO`, `CALL_VIDEO_BYTES_PER_SEC`,
  `CALL_MONTHLY_EGRESS_GB`, `CALL_FICHAS`, `CALL_FICHAS_SEED`, `CALL_REDIS_URL` e `CALL_STATE_FILE`.
- Apps: sem pedido de fichas nem leitura do `estado.json` do site. Ao abrir, a base cifrada perde a
  tabela `fichas` e as chaves `fichas:*` (`secure_delete`). Nenhuma string das fichas (nunca mostraram
  texto).
  `VideoPause::RelayLimit` passa a `NoDirect` (só com a câmara autorizada) e sai `limited_by_relay`
  da FFI (as apps não os usavam).
- Deploy: o `setup-ec2.sh` deixa de tratar da semente, do `fichas.env`, do `sealer-estado.json` e
  das variáveis do vídeo; o `limites.conf` fica só com os limites das salas. Site: o `estado.json`
  fica só com a versão.
- Documentação: `docs/protocol-spec.md` (secção 18, vídeo, e secção 24, só a compatibilidade),
  `docs/threat-model.md`, `docs/known-limitations.md` (46), `docs/aws-orcamento.md`, README, ROADMAP.
  Sai `docs/privacidade-fichas-rascunho.md`.
- **Sai a opção "Relay (teste)"** das duas apps, com o código: a linha nas Definições, o ecrã, as
  "Opções de teste" do primeiro ecrã (URL e chave do relay) e o `RELAY_EDITABLE` (Android) /
  `Config.relayEditable` e `QTRelayEditable` (iPhone). O relay é sempre o fixo do
  `gradle.properties` / `Relay.xcconfig`, também nas builds debug, e a chave de pinning passa a ser
  obrigatória em todas as builds. As apps apagam o relay de teste que uma build debug antiga tivesse
  guardado. Saem das 10 línguas as strings que ficaram sem uso: `t_relay_teste`,
  `t_opcoes_de_teste`, `t_esconder_opcoes_de_teste`, `t_url_do_relay`,
  `t_chave_publica_do_relay_pinning`, `t_guardar_e_voltar_a_ligar` e `t_o_relay_tem_de_usar_wss`.
  No iPhone, os registos da chamada no ecrã continuam só na Debug (`#if DEBUG`).
- Android e iPhone: versão 0.9.53 (build 60).

### 30 de outubro de 2026: Comunidade, a lista das pessoas com medalha (Android e iPhone 0.9.52, Sealer Control 2.2)

- Ecrã novo **Comunidade** nas duas apps (Definições): todas as pessoas com medalha, incluindo a
  Troll e a Legacy, só com a foto e a medalha (sem nome, sem iniciais e sem ID), com o aro ou o
  escudo como nos contactos. Ordem: por medalha e, dentro de cada uma, por ordem de entrada. Sem
  foto, o avatar por omissão com a medalha. Strings novas nas 10 línguas (`community_title`,
  `community_subtitle`, `community_empty`).
- Envio automático da foto (só quem tem medalha): a miniatura dos contactos, com o certificado da
  medalha, autenticada com a chave de identidade da conta e selada para a chave do operador como
  as denúncias. Só quando é preciso: primeira vez, medalha ou foto nova (tirar a foto também) e,
  sem mudanças, no máximo uma vez por semana. Um certificado copiado de outra pessoa é recusado.
- Relay: `CommunitySubmit` (até 3 por conta e por dia, sem o ID de quem enviou, 30 dias),
  `CommunityVersion`, `CommunityIndex` e `CommunityPhotos` para as apps, e pedidos do operador
  para recolher, apagar e publicar. A lista (índice e fotos) fica 8 dias, com os comandos do Redis
  que o ACL já permitia. Apps antigas sem mudanças.
- Tráfego: as apps só pedem a lista ao abrir o ecrã, primeiro só o número; com o número igual não
  desce mais nada, e com uma lista nova só descem as fotos novas. Sem lista no relay, mostram a
  última guardada.
- Núcleo: `core/src/community.rs` (envio, índice assinado com a chave das medalhas e cifrado,
  fotos identificadas pelo hash) e `ffi/src/community.rs`. Ferramenta do operador
  `server/examples/comunidade.rs` (recolher e publicar, nos dois relays, com estado local cifrado);
  a ferramenta `badge` passa a escrever o registo das medalhas entregues (`<db>.registo`), para uma
  medalha retirada sair da lista.
- Sealer Control 2.2: página Comunidade com "Atualizar Comunidade", "Não mostrar" por membro e
  "Publicar lista" (repositório sealer-control, patch à parte).
- Site: política de privacidade (quem tem medalha aparece com foto e medalha, sem nome nem ID) e
  "Como funciona", nas 10 línguas. `docs/protocol-spec.md` (secção 26) e `docs/threat-model.md`
  (a chave da lista vai dentro das apps: aceite).
- iPhone: versão 0.9.52 (build 59), igual ao Android.

### 30 de outubro de 2026: chamadas do Android no sistema de chamadas do telemóvel (Android 0.9.52)

- As chamadas de voz e as videochamadas do Android entram no sistema de chamadas do telemóvel
  (Jetpack Core-Telecom 1.0.1, chamadas "self-managed"), como o CallKit no iPhone. Android 8 ou
  mais recente; no Android 7 fica tudo como antes.
- Auricular Bluetooth e carro: o som vai para lá e os botões atendem, recusam e desligam. O nome
  mostrado ao sistema, ao carro e ao Bluetooth é a alcunha do contacto; com o bloqueio da app,
  ou sem nome, "Chamada SEALER" / "Videochamada SEALER". Silenciar na app ou no carro fica igual
  nos dois lados.
- Chamada normal ao mesmo tempo: atender uma chamada GSM durante uma chamada Sealer termina a do
  Sealer dos dois lados (sem chamadas em espera, como no iPhone). Uma chamada Sealer que chega
  durante uma GSM aparece sem o toque alto por cima da conversa. Ligar a partir do Sealer
  durante uma GSM mostra o erro de sempre ("Não foi possível ligar: Ocupado noutra chamada").
- Privacidade: nada entra no registo de chamadas do telemóvel; o endereço dado ao sistema é
  sempre "sealer:SEALER", nunca o ID do contacto.
- Saída do som: sem Bluetooth, o botão do altifalante fica igual. Com Bluetooth ou auscultadores,
  o mesmo botão abre uma lista curta (Telemóvel, Altifalante, o nome do aparelho). Strings novas
  nas 10 línguas (`call_route_title`, `call_route_phone`, `call_route_headphones`).
- Código: `SystemCalls.kt` (novo) entre o `MainViewModel` / `SealerCore` e o Telecom;
  `CallRoutes.kt` (lógica das saídas, com testes em `CallRoutesTest`). Com a chamada no sistema,
  o modo de chamada, o foco e a saída do som são do Telecom (`CallAudioIO` e o "tuu... tuu" já
  não mexem neles). O toque próprio, a notificação de ecrã inteiro e o `CallService` continuam.
- Com a app fechada, atender no auricular ou no carro abre a app para atender, como o "Atender"
  da notificação (o microfone precisa da app no ecrã).
- Relay e protocolo sem mudanças.

### 30 de outubro de 2026: a ferramenta das medalhas mostra porque é que o relay recusou

- `server/examples/badge.rs`: quando o relay recusa o envio, a ferramenta escreve a razão que o relay
  deu (por exemplo, `[nao enviado: recusado pelo relay: destinatario desconhecido]`). Antes só
  escrevia "recusado". Apps, relay e Sealer Control sem mudanças.

### 29 de outubro de 2026: medalhas Bug Hunter, Support, Troll e Legacy (iPhone e Android 0.9.51)

- Quatro medalhas novas: 6 Bug Hunter, 7 Support, 8 Troll e 9 Legacy (`core/src/badge.rs`,
  `BADGE_MAX` passa a 9). As três primeiras são moedas como as outras, com aro da mesma cor
  (vermelho, laranja e azul).
- A Legacy é diferente: um escudo prateado à volta da foto, sem aro nem medalha no canto
  (`badge_shield_legacy.png` no Android, `BadgeShieldLegacy` no iPhone). O escudo é 1,25 vezes o
  avatar e passa um pouco para fora (sobretudo em baixo), sem mudar o tamanho do avatar; a foto
  fica com 75% do avatar (no aro são 84%). O ponto verde de online vai para cima, como com o aro.
- Apps até à 0.9.50: recusam os números 6 a 9 e não mostram nada (nem erro).
- Ferramenta do emissor (`server/examples/badge.rs`): `--badge bughunter`, `support`, `troll` e
  `legacy`. Teste novo em `ffi/tests/badges.rs` (as quatro chegam aos contactos; o 10 é recusado).
- Site (Como funciona): as quatro na lista das medalhas e uma frase sobre o escudo da Legacy, nas
  10 línguas (chaves `como.40` a `como.43`).

### 29 de outubro de 2026: relay com menos idas ao Redis

- O perfil de CPU (29/10/2026, 25 000 ligados) mostrou o vaivém entre o relay e o Redis a custar ~35% do
  CPU (os dois lados somados): cada mensagem fazia vários pedidos seguidos.
- Envio (`SendMessage`, e as mensagens vindas de outro relay): um só pedido em vez de dois. As
  verificações vão junto com a escrita; se alguma falhar, a entrada é retirada e o prazo da fila (se já
  existia) é reposto. Os ids saem de blocos de 1000 reservados no Redis, em vez de um `INCR` por mensagem.
- O bundle (~1,4 KB) deixa de ser lido só para saber se existe (`EXISTS`). Quem envia é verificado uma
  vez por ligação (na autenticação, no mesmo pedido que a verificação de conta banida).
- Confirmação (`AckMessages`): um pedido em vez de dois, com a página que a ligação acabou de entregar.
- Registo do bundle: uma transação (bundle, one-time prekeys e cópia de recuperação) em vez de cinco a
  sete pedidos, sem repetir a verificação de conta banida. Anexo novo (`BlobCreate`) e premium: um pedido
  cada.
- ACL do Redis (`deploy/redis/qt-redis.conf`): `+exists +ttl`, só leitura. Apps e protocolo sem mudanças.

### 29 de outubro de 2026: Redis do relay só com o AOF

- O perfil de CPU (29/10/2026, 25 000 ligados) mostrou o Redis a gravar o disco de duas maneiras ao mesmo
  tempo: o AOF e o `dump.rdb` (regras `save` por omissão), cada um com ~5% de CPU.
- `deploy/redis/qt-redis.conf`: `save ""` (só o AOF, que já era o que o Redis lia ao arrancar) e
  `auto-aof-rewrite-min-size 256mb` (reescreve o AOF menos vezes). `.gitignore`: `appendonlydir/`.
- Apps, protocolo e relay sem mudanças. Entra com o `setup-ec2.sh`, que reinicia o Redis.

### 29 de outubro de 2026: relay com menos memória por ligação

- Teste de carga (2 CPUs, 1,8 GB): o relay aguentava ~21 000 pessoas ligadas antes de ficar sem memória.
  O que mais pesava era a biblioteca de WebSocket (tungstenite 0.24): em cada ligação, um bloco fixo de
  4 KB e o espaço da maior mensagem que já tinha passado (até ~64 KB depois de um anexo), para sempre.
- Agora usa uma cópia do tungstenite 0.24 (`vendor/tungstenite`, ver `SEALER.md`) que liberta esses
  buffers quando ficam vazios. Medido com 8 000 ligações, texto e áudios: de 44 KB para 8 KB por ligação
  parada e de 88 KB para 12 KB depois de tráfego, com as mesmas entregas e tempos.
- Protocolo e apps sem mudanças. Só ganham os relays atualizados (Estocolmo e Espanha, com
  `QT_BRANCH=main`).

### 29 de outubro de 2026: documentação revista, com a federação em destaque

- README (PT e EN): apps 0.9.50, secção "Federação: dois relays, uma só rede", os dois relays
  (`relay.sealerapp.com` e `relay-es.sealerapp.com`), uma só ligação por app desde a 0.9.45 e o
  `setup-ec2.sh` do ramo `p2p-teste`. No README inglês, videochamadas também no iPhone.
- Esquema da arquitetura (`docs/arquitetura.svg`, `docs/architecture.svg`): dois relays em federação, a
  ligação fechada entre eles e o site em sealerapp.com.
- `ROADMAP.md`: federação feita (fases 1 a 4) e o que falta. `docs/known-limitations.md` (14) e
  `docs/threat-model.md`: o que a federação muda (o relay par, a confiança entre relays, a conta largada).
- Endereços do sealer.pt trocados pelo sealerapp.com em `docs/google-play.md`, `docs/threat-model.md`,
  `docs/protocol-spec.md`, `docs/performance.md`, `ios/README.md` e no rascunho das fichas.

### 29 de outubro de 2026: videochamadas no iPhone (iPhone e Android 0.9.50)

- O iPhone passa a ter videochamadas, como o Android: botão da videochamada na conversa (só para contactos
  com vídeo), pedido "[Nome] quer mostrar vídeo" (Ver / Ligar a minha câmara / Agora não), trocar câmara,
  vídeo pelo relay com as fichas anónimas (chaves verificadas no `estado.json` do site) e vídeo direto entre
  contactos mútuos (teto conforme Wi-Fi ou dados móveis). Câmara e H.264 em hardware (AVFoundation e
  VideoToolbox); a cifra, os pacotes e o débito continuam no núcleo Rust (feature `video` na build iOS).
- Ecrã de chamada do iPhone com o desenho do Android: "Sealer" e o tempo no topo, barra de controlos,
  desligar vermelho, controlos que se escondem com vídeo, ecrã sempre aceso com vídeo e apagado ao ouvido
  só de voz. Toque longo no tempo: diagnóstico da chamada com "Copiar diagnóstico".
- iPhone: as chamadas entre contactos mútuos passam a ligar direto (faltava `call_contacts_sync`).
- iPhone: aviso quando o microfone deixa de funcionar a meio da chamada e, ao recuperar a conta, o que
  volta e o que não volta.
- A câmara só fica parada com a app fora do ecrã (a voz continua). Android sem mudanças além da versão.
- As duas apps voltam a ter o mesmo número de versão (0.9.50; iPhone build 57, igual ao versionCode).

### 29 de outubro de 2026: relay de Espanha ligado às apps (Android 0.9.47 e iPhone 0.9.34)

- Segundo relay da federação: `espanha` (`wss://relay-es.sealerapp.com`, eu-south-2), com a chave fixada nas
  duas apps (`qtRelaysOutros` / `QT_RELAYS_OTHERS`). O principal continua a ser Estocolmo.

### 29 de outubro de 2026: federação de relays, fases 3 e 4 (apps Android 0.9.46 e iPhone 0.9.33)

- As apps têm a lista dos relays da federação (o principal e os outros, com as chaves fixadas) e guardam o
  relay da conta. Contas novas ficam no principal, ou no seguinte se o principal estiver fechado a contas
  novas (`RELAY_NOVAS_CONTAS=0`). Uma conta recuperada noutro telemóvel volta sozinha ao seu relay.
- O operador trata premium, quotas e banimentos de contas de qualquer relay a partir de um só.
- Chamadas entre contas de relays diferentes: as duas apps usam o relay das chamadas de quem liga (o nome vai
  na oferta, só quando não é o principal). Todos os relays usam a mesma semente das fichas do vídeo.
- Sem outros relays na build (`qtRelaysOutros` vazio), tudo funciona como antes.
- Ver `docs/protocol-spec.md`, secções 25.2 e 25.3.

### 29 de outubro de 2026: federação de relays, fase 2 (só relay)

- Mensagens, bundles, anexos e cópias de recuperação entre contas de relays diferentes: o relay de quem pede
  encaminha para o relay de casa da conta. As apps não mudam.
- O relay de casa só sabe que a mensagem veio do outro relay (sealed sender igual). Relay de casa em baixo:
  a mensagem fica na outbox da app e segue depois.
- Ver `docs/protocol-spec.md`, secção 25.1.

### 29 de outubro de 2026: federação de relays, fase 1 (só relay)

- Relays de uma lista fechada (`/etc/qt/federacao.json`) ligam-se entre si por `wss://<relay>/fed`, com
  Noise e a chave fixada dos dois lados; o HAProxy só deixa entrar os IPs da lista. Sem ficheiro, o relay
  funciona como antes.
- Registo único: uma conta nova só se regista se nenhum outro relay a tiver (`Error{"conta noutro relay"}`)
  nem a tiver banida. Com um relay em baixo regista na mesma e verifica quando ele voltar.
- Descoberta de contas noutros relays (para a fase 2, mensagens entre relays), com cache só em memória.
- Apps e protocolo das apps sem mudanças. Ver `docs/protocol-spec.md`, secção 25.

### 29 de outubro de 2026: uma só ligação ao relay (apps Android 0.9.45 e iPhone 0.9.32)

- Cada app aberta passa a usar **uma** ligação ao relay em vez de três (pedidos, espera por mensagens e
  anexos). Cada ligação custa ~40 KB no servidor: a mesma máquina aguenta o triplo das pessoas ligadas.
- Os pedidos vão em fila pela mesma ligação e o relay responde pela mesma ordem. A espera por mensagens
  continua: quando sai outro pedido (um envio, um bloco de anexo), o relay acaba a espera, responde com o
  que houver e trata o pedido a seguir. O relay e o protocolo não mudam; apps antigas e novas funcionam
  com o mesmo relay.
- Anexos em lotes de 4 blocos (~160 KB, antes 8): durante um envio grande, uma mensagem espera no máximo
  um lote. Entre lotes a app continua a receber.
- Menos tráfego com a app aberta: um só ping de 25 em 25 s em vez de três.
- Endereços do sealer.pt (desligado) trocados pelo sealerapp.com: chaves das fichas do vídeo
  (`https://sealerapp.com/estado.json`; sem elas o vídeo pelo relay pausava a partir de novembro), links dos
  Termos e da Privacidade, email do "Reportar erro de tradução" (`privacidade@sealerapp.com`) e a verificação
  das chaves no `setup-ec2.sh`.

### 28 de outubro de 2026: diagnóstico das chamadas e acertos (app Android 0.9.44)

- Diagnóstico para os testers (fase 5c): `CallAudio.diagnostic()` no núcleo dá os números da chamada
  (tempo em direto e pelo relay, perdas da voz e do vídeo, débito, resolução, pausas do vídeo, mudanças
  de rede, fichas), sem IPs, IDs nem conteúdo, e o resumo final depois de desligar. Fica só no telemóvel.
  Android: toque longo no tempo da chamada abre o painel com "Copiar diagnóstico"; nas Definições,
  "Copiar diagnóstico da última chamada"; registos discretos no logcat (rede, caminho, codificador).
  Ligado por omissão nas builds de teste; desligar antes da Play Store com `-PqtCallDiag=false`.
- Vídeo: se o codificador recusar o formato (ex. `c2.android.avc.encoder` com
  `KEY_PREPEND_HEADER_TO_SYNC_FRAMES`), a app tenta os outros codificadores do telemóvel (hardware
  primeiro) e formatos mais simples; o SPS/PPS vai à frente de cada imagem completa.
- Sai "Cifrada ponta a ponta" do cabeçalho da conversa, do ecrã da chamada e do cabeçalho dos grupos
  (agora só "N pessoas"), no Android e no iPhone, nas dez línguas. O site continua a explicar a cifra.
- Teste `ligar_tocar_atender_falar_desligar`: passa também com `--features video` (ignora o evento de
  vídeo ao atender, que é o comportamento certo).

### 27 de outubro de 2026: pessoas online (só relay)

- Relay: `AdminOnline` (só o operador) diz quantas pessoas têm a app ligada agora (contas, não
  ligações; sem o operador). Só o número, contado em memória. Ferramenta:
  `--example denuncias -- online --key ... [--json]`.

### 27 de outubro de 2026: relay em relay.sealerapp.com (app Android 0.9.43, iPhone 0.9.31)

- As apps passam a ligar a `wss://relay.sealerapp.com` (o mesmo servidor e o mesmo pin). As versões
  anteriores deixam de chegar ao relay e têm de ser atualizadas.

### 24 de outubro de 2026: videochamadas com a app fechada ou o ecrã bloqueado (0010, app Android 0.9.42)

- Videochamada a chegar com a app fechada ou o telemóvel bloqueado: o aviso de ecrã inteiro diz
  "Videochamada de [Nome]" (com o botão de atender com a câmara). Se a indicação de vídeo chegar
  depois da chamada, o aviso muda sem voltar a tocar do início. Com o bloqueio da app só aparece
  "Chamada SEALER", sem dizer se é de voz ou de vídeo.
- Por cima do bloqueio do telemóvel o ecrã da chamada passa a ter vídeo: o pedido "[Nome] quer mostrar
  vídeo" e o vídeo do outro aparecem; a minha câmara só liga se eu carregar no botão.
- App em segundo plano ou ecrã bloqueado a meio da chamada: a voz continua; a minha câmara fecha em
  silêncio e volta sozinha quando a app volta ao ecrã, se estava ligada. Do outro lado vê-se só a
  minha foto ou a inicial, sem texto. A pausa é só deste lado (`CallAudio.video_hold` no núcleo):
  nada sai para a rede e o outro lado não recebe aviso nenhum.
- Serviço da chamada com os tipos `phoneCall|microphone|camera` (Android 14+; câmara só com a câmara
  ligada). Arranca só com a app no ecrã; depois os tipos mudam no próprio serviço, sem novo arranque em
  segundo plano. Nunca para antes de entrar em primeiro plano. Permissões novas:
  `FOREGROUND_SERVICE_PHONE_CALL`, `MANAGE_OWN_CALLS` e `WAKE_LOCK`.
- Sensor de proximidade só nas chamadas de voz (ecrã apaga ao encostar ao ouvido, nunca com vídeo nem em
  altifalante). Com vídeo, o ecrã fica sempre aceso.
- Sem strings novas e sem bibliotecas novas.

### 24 de outubro de 2026: política de privacidade, site e alarme de orçamento (0008)

- Política de privacidade (dez línguas): secção nova "Chamadas e videochamadas" (ligação direta entre
  contactos mútuos, e nesse caso cada um vê o IP do outro; nos outros casos pelo servidor, sem partilhar
  o IP; vídeo só para quem aceita), recuperação com as 12 palavras (contactos sim, mensagens antigas e
  grupos não) e idade mínima de 16 anos. Saem os itens das funções pagas (guardados em
  `docs/privacidade-fichas-rascunho.md`, que não entra no site).
- Site (dez línguas): chamadas de voz e videochamadas, ligação direta entre contactos; sai "chamadas
  sempre pelo servidor" (também do README).
- `docs/aws-orcamento.md`: guia do orçamento mensal no AWS Budgets e do alarme do tráfego de saída da EC2.

### 24 de outubro de 2026: novo ecrã da videochamada (0007, app Android 0.9.38)

- Só Android, só a interface da chamada (núcleo, relay e fichas sem mudanças). O esboço serviu de
  referência de layout; o logótipo e as cores são os da app (tema 0.9.11, claro e escuro), com
  "Sealer" em Manrope Bold, parado.
- Vídeo do outro em ecrã inteiro e a minha câmara num canto, com cantos arredondados.
- Topo com o logótipo e "Sealer" à esquerda e o tempo da chamada à direita.
- Barra de controlos flutuante com 4 botões redondos: microfone (silenciar), altifalante (ouvido ou
  altifalante), câmara (ligar ou desligar) e trocar câmara; por baixo, o desligar vermelho com o
  auscultador.
- Com vídeo, o topo e os controlos desaparecem ao fim de 3,5 s sem toques e voltam com um toque
  (o Android pode pedir mais tempo nas definições de acessibilidade; com o TalkBack ficam sempre).
  Só de voz: sempre visíveis.
- Sem avisos de estado: só "À espera de [Nome]…"; vídeo do outro parado mostra a foto ou a inicial.
- Acessibilidade: cada botão com descrição nas dez línguas (string nova `call_camera`); os de
  ligar/desligar são anunciados como interruptores. Ícones desenhados no código (sem bibliotecas novas).

### 23 de outubro de 2026: fichas anónimas do vídeo pelo relay, P2P automático e chamada sem avisos (0006, app Android 0.9.37)

- Quota de vídeo pelo relay por conta e por mês, configurável no servidor, sem o relay saber quem está na sala:
  fichas Privacy Pass com VOPRF (RFC 9497, ristretto255-SHA512, crate `voprf` 0.5), emitidas em lote
  e cegas pelo relay das mensagens (só conta quantas deu a cada conta por mês) e gastas na sala
  anónima por quem envia vídeo pelo relay (1 ficha = 1 minuto). Gasto duplo no Redis por mês
  (utilizador `qt-fichas`, só `SET` em `fichas:*`), chave nova por mês derivada de uma semente,
  prova DLEQ verificada na app, fichas guardadas na base cifrada e apagadas quando o mês acaba.
  Sem fichas: só voz pelo relay; em ligação direta o vídeo continua sem gastar nada. Apps antigas:
  só voz pelo relay. `docs/protocol-spec.md` secção 24.
- Chave das fichas de cada mês publicada fora do relay, no site (`https://sealer.pt/estado.json`,
  Cloudflare; gerada no deploy com `qt-relay chaves-fichas 24`). A app recusa emitir se a chave do
  emissor for outra e não gasta fichas se a chave da sala for outra (ataque de partição de chaves);
  sem a chave do site, o vídeo pelo relay fica em pausa (voz e direto normais).
- O mês dito pelo relay só se segue se for o vizinho do relógio do telemóvel, a menos de 48 h da
  viragem, e o motor só recebe as chaves publicadas desses meses (partição pelo mês: um relay que
  dissesse a cada conta um dos 24 meses publicados separava-as em grupos).
- Relay: `IssueFichas`, `AdminSetFichasQuota` (quota por conta, pelo operador),
  `RELAY_FICHAS_POR_MES`, `FICHAS_SEED_PATH`; relay das
  chamadas com `CALL_FICHAS`, `CALL_FICHAS_SEED` e `CALL_REDIS_URL` (`/etc/qt-call-relay/fichas.env`).
  `setup-ec2.sh` cria a password do `qt-fichas` e mantém os valores entre atualizações.
- Ligação direta automática entre contactos mútuos e não bloqueados, já na primeira chamada
  (substitui a regra do 0005): saem a janela de aviso, a opção "Ligação direta nas chamadas" do menu
  da conversa, `call_direct_choice`, `call_set_direct_choice` e a chave `call:direct:` (apagada). Os
  endereços continuam a sair só depois de atender e só entre contactos mútuos; bloquear, apagar ou
  tirar dos contactos volta logo ao relay. Nessas chamadas cada lado vê o IP do outro.
- Ecrã da chamada sem avisos de estado: saem "Vídeo em pausa · Rede fraca" e "Vídeo em pausa · Limite
  do mês atingido" (dez línguas). Com o vídeo do outro parado, só a foto ou a inicial dele, sem texto.
  Sai também "[Nome] não quis ver o vídeo": a câmara de quem enviou desliga-se em silêncio. Fica só
  "À espera de [Nome]…".
- Testes: fichas válidas uma vez, repetidas, de outro mês, forjadas e de outra chave recusadas; teto
  por conta e por mês (também com pedidos ao mesmo tempo); o relay não liga resgate a emissão; sem
  fichas a voz continua e o vídeo pausa; ligação direta não gasta; relay antigo; ACL do `qt-fichas`;
  chave diferente da do site na emissão e na sala recusada; sem a chave do site falha fechada.

### 23 de outubro de 2026: ligação direta entre contactos, videochamada e limpezas

- Ligação direta (P2P) sem verificação, QR nem códigos: liga quando os dois se têm nos contactos e
  disseram OK na janela de aviso desse contacto. Se um dos lados não tiver o outro nos contactos, ou
  disse "Agora não", a chamada vai sempre pelo relay. Apagar, bloquear ou tirar o contacto da lista a
  meio de uma chamada volta logo ao relay. Os endereços continuam a sair só depois de atender, dentro
  do ratchet (ops 5 e 6, sem mudanças no formato).
- Núcleo: `call_contacts_sync` (lista de contactos da app e anúncio "tenho vídeo" uma vez por
  contacto, op 7 fora de uma chamada), `call_direct_choice` / `call_set_direct_choice`,
  `call_video_contacts`, `call_start_video` e `call_is_video`. Saem `calls_relay_only`,
  `set_calls_relay_only` e `contact_verified` (e a chave `verified:`).
- Janela de aviso, uma vez por contacto e nunca durante uma chamada: quem liga vê-a ao carregar em
  ligar; quem recebe, depois de desligar a primeira chamada atendida. [OK] [Agora não]. No menu da
  conversa, "Ligação direta nas chamadas" muda a resposta. Textos em dez línguas.
- Botão de videochamada na conversa, ao lado do de ligar, só para contactos com vídeo. Começa com a
  minha câmara ligada; quem recebe vê "Videochamada de [Nome]" e, ao atender, escolhe se vê o vídeo.
- Limpezas: sai "Chamadas sempre pelo servidor" das Definições e a faixa "Qualidade limitada ·
  Verifica o contacto"; pausas do vídeo com "·" ("Vídeo em pausa · Rede fraca", "Vídeo em pausa ·
  Limite do mês atingido"); na recuperação de conta aparece antes de recuperar "Vais recuperar os teus
  contactos. As mensagens antigas e os grupos não são recuperados."
- Testes: nenhum endereço sai antes de atender nem para quem não é contacto mútuo com OK (só um lado
  com o outro nos contactos, sem resposta à janela, "Agora não"); bloquear, apagar e sair da lista a
  meio voltam ao relay; videochamada; anúncio do vídeo.

### 23 de outubro de 2026: videochamadas na app Android, com a app aberta (fase 5b)

- Botão "Ligar câmara" na chamada (só se os dois lados tiverem vídeo). Ligar a câmara é um pedido: o
  outro vê "[Nome] quer mostrar vídeo" com Ver, Ligar a minha câmara e Agora não; quem pediu vê-se a si
  próprio com "À espera de [Nome]…". Nada do vídeo sai antes de o outro aceitar.
- Vídeo do outro em ecrã inteiro e a minha câmara num canto; trocar câmara frontal/traseira; desligar
  a câmara. Mensagens de pausa por rede fraca, de limite do mês pelo servidor e de qualidade limitada
  (contacto por verificar).
- Câmara (Camera2) e H.264 em hardware (MediaCodec): débito, resolução, imagens/s e imagens completas
  seguem o que o núcleo pede; rotação enviada com cada imagem; descodificador que recomeça e pede
  uma imagem completa se falhar. Rede Wi-Fi ou dados móveis passada ao núcleo (teto da ligação direta).
- FLAG_SECURE no ecrã da chamada; serviço da chamada também do tipo câmara com a câmara ligada; a
  permissão da câmara só é pedida ao carregar no botão. Textos em dez línguas.
- A build Android passa a anunciar vídeo (feature `video`).

### 23 de outubro de 2026: videochamadas, núcleo e relay (fase 5a)

- Vídeo dentro da chamada de voz, pelo mesmo caminho (direto ou relay, com a mesma duplicação e queda
  para o relay). A chamada começa sempre só com voz; o botão da câmara só aparece se os dois lados
  tiverem vídeo (ops 7 e 8 no ratchet). O vídeo de quem liga a câmara só sai quando o outro aceita ver.
- Chaves próprias (separação de domínio), pacotes até 1225 bytes com o tamanho arredondado a 256,
  remontagem com espera pela cópia do relay, pedidos de imagem completa, sincronização com o áudio e
  controlo de débito no Rust: até 360p pelo relay (240p com o travão do mês), até 1080p em Wi-Fi e 720p
  em dados móveis na ligação direta. O áudio tem sempre prioridade. Abaixo dos 240p o vídeo pausa e
  volta sozinho.
- Relay: aceita vídeo com débito limitado por lado (150 kB/s), travão do mês (80 GB, depois 240p para
  todos, contador em `/var/lib/qt-call-relay/mes`) e a porta para o teto por conta (fichas anónimas
  numa sessão própria). Variáveis no drop-in `limites.conf` do `setup-ec2.sh`, que mantém os valores
  já postos.
- Só uma build com o ecrã do vídeo anuncia vídeo (feature `video`, ligada na build Android pela 5b); o
  iPhone continua só com voz (`case .video`).
- Testes e2e: vídeo pelo relay e pelo direto, nos dois sentidos com 150 ms de atraso, queda do direto a
  meio, perda real de 10 %, pausa e volta, teto do relay, travão do mês, relay antigo, rede estreita
  e consentimento; FFI: pedido, "Agora não", ligar a minha câmara, app antiga.

### 23 de outubro de 2026: jitter buffer espera pelos atrasados até ao alvo

- Os pacotes que chegam tarde demais passam a contar para o atraso alvo, e um frame em falta com outro
  já à frente espera (até ao alvo) em vez de ser dado logo por perdido. Simulação com +0 a 150 ms de
  variação durante 4 s: 37 frames perdidos passam a 4, com +20 ms de atraso no pico; o atraso volta ao
  normal no mesmo tempo (~5 s). Com a ligação direta em duplicado, as cópias pelo relay (+150 ms)
  passam a tocar em vez de chegarem tarde.
- Muda o comportamento da 0.9.5 em todas as chamadas, também só pelo relay: testar em telemóveis reais.

### 23 de outubro de 2026: chamadas, ligação direta (P2P, fase 5, primeira parte)

- A chamada começa sempre pelo relay. Depois de atender, se os dois lados tiverem o outro verificado e
  nenhum tiver "Chamadas sempre pelo servidor", os telemóveis trocam endereços dentro do ratchet (ops 5
  e 6) e o áudio passa a ir direto. Nenhum endereço sai antes de atender nem para quem não verificou.
- Endereços IPv4 e IPv6 (socket dual), incluindo o IP público visto pelo relay (pedido 0x22, só para
  salas abertas, resposta mais pequena que o pedido).
- Se o direto falhar, o áudio vai pelos dois caminhos (direto e relay) a partir de 150 ms sem áudio do
  outro lado ou 250 ms sem resposta às sondas (uma a cada 100 ms), e fica só no relay aos 3 s (1 sonda/s
  até o direto voltar). Corte medido numa queda brusca: ~0,2 s (antes ~0,75 s).
- A cópia repetida é descartada pela janela anti-repetição e não conta como recebida, atrasada nem
  perdida.
- Compatível com apps e relays antigos: sem suporte dos dois lados, tudo pelo relay como hoje.
- App Android: interruptor "Chamadas sempre pelo servidor" em Definições, Privacidade (dez línguas).
- Ainda não há forma de verificar um contacto na app (parte 2) nem a janela de aviso (parte 3): até lá
  as chamadas continuam todas pelo relay.

### 23 de outubro de 2026: texto das 12 palavras (apps Android 0.9.32 e iPhone 0.9.30)

- Ecrã das 12 palavras: saiu "Não tires captura de ecrã nem as guardes no telemóvel" (dez línguas), que
  contradizia o botão "Copiar as palavras". Fica "Escreve estas palavras em papel, por esta ordem."

### 23 de outubro de 2026: recuperar a conta e menu Idioma (app Android 0.9.31)

- Ao voltar à app no ecrã de recuperar a conta (ex.: depois de ir copiar as 12 palavras) aparecia um erro:
  o refrescamento ao voltar pedia os grupos sem haver identidade. Agora só refresca com identidade.
- Menu Idioma: a lista desliza. Com dez línguas não cabia em ecrãs mais baixos (hindi e árabe esmagados,
  o árabe sem nome).

### 23 de outubro de 2026: altifalante antes de atender (app Android 0.9.30)

- O botão do altifalante passa a funcionar enquanto se chama: o telemóvel entra em modo de chamada logo
  com o "tuu... tuu" (auricular por omissão, altifalante a pedido; as outras apps baixam o som). Antes o
  botão só mudava o texto.
- A escolha feita antes de atender mantém-se depois de atender (antes o ecrã dizia altifalante e o som
  saía pelo auricular). Sem resposta, o som do telemóvel volta ao normal. O iPhone já fazia assim.

### 23 de outubro de 2026: pedidos dos testers (apps 0.9.29)

- Chamadas: o "tuu... tuu" de quem liga começa logo ao ligar (antes só com o aviso "a tocar" do outro
  telemóvel, que não chega com o bloqueio da app ligado nem sem rede) e mais alto (Android; o iPhone já
  fazia assim).
- Chamadas: minimizar (seta no ecrã da chamada ou "voltar" no Android). A app fica usável e uma barra verde
  no topo, com o nome e o tempo, volta à chamada. A chamada a chegar nunca fica escondida.
- Chave de recuperação: botão "Copiar as palavras". A cópia vai marcada como sensível e a área de
  transferência é limpa ao fim de 1 minuto (iPhone: só neste aparelho, sem a área partilhada com o Mac).
- Links nas mensagens: clicáveis, com confirmação que mostra o nome do site antes de sair da app. Só
  `http(s)://` e `www.`. Sem pré-visualização (decisão do Nuno: fica assim, por privacidade). O toque longo numa mensagem de texto tem
  "Copiar mensagem" e "Copiar link", também com o contacto bloqueado ou fora do grupo (aí sem reações).
- Badges e fotos depois de recuperar a conta: quem recebe uma sessão nova de um contacto esquece o que lhe
  tinha enviado (`badge:sent:`, `profile:sent:`), e a foto e o badge voltam a seguir (antes o contacto que
  reinstalou ficava sem ver os badges dos outros). Teste `depois_de_recuperar_os_contactos_voltam_a_mandar_o_badge_e_a_foto`.
  Só resulta quando os contactos também tiverem a 0.9.29.
- Cópia de recuperação refeita quando o meu badge chega ou muda (antes só com mudanças nos contactos ou
  bloqueados, e o badge podia ficar fora da cópia).

### 23 de outubro de 2026: árabe e correções das traduções (apps 0.9.25 e site)

- Apps e site em árabe (`values-ar`, `ar.lproj`, `site/src/i18n/ar.js`): árabe padrão, neutro sempre que
  possível (masculino genérico só onde não dá), algarismos 0-9 (Android `Lang.locale()` com `nu=latn`,
  iPhone `Lang.numbersLocale`). Nomes do exemplo: آنا e روي. Palavra para apagar tudo: حذف (aceita حدف e DELETE).
- Da direita para a esquerda: `android:supportsRtl`, cabeçalho curvo e estrelas ao espelho (Android e
  iPhone), setas `chevron.backward/forward`, gesto de voltar do lado certo no iPhone, avião de enviar
  virado, sem espaçamento entre letras nos rótulos. Site: `<html dir="rtl">`, CSS com propriedades lógicas,
  setas do teclado trocadas no "Como funciona".
- Texto misto (bidi): número de segurança, tamanhos de ficheiros e IDs ficam na ordem certa (LRM/isolados).
- Código de acesso: algarismos de qualquer teclado (٠-٩, ०-९) passam a 0-9, para o mesmo código funcionar
  com qualquer teclado.
- Palavra de confirmação: ignora tatweel, sinais de vogal e marcas de direção.
- Correções das traduções (japonês, chinês, hindi): 管理者をやめる, 表情回应, 对方/接收者 no site,
  एक पल रुकें…, न विज्ञापन, न एनालिटिक्स.
- Revisão independente do árabe (tradução e RTL) aplicada antes do patch.

### 22 de outubro de 2026: japonês, chinês simplificado e hindi (apps 0.9.24 e site)

- Apps Android e iPhone em nove línguas: `values-ja`, `values-zh-rCN`, `values-hi` e as pastas
  `ja`, `zh-Hans` e `hi.lproj` do iPhone (geradas por `scripts/strings-ios.py`). Japonês na forma
  educada (です・ます), chinês com 你 (您 nas páginas legais do site), hindi sempre com आप.
- Definições → "Reportar erro de tradução": abre o email para privacidade@sealer.pt com a língua e a
  versão da app no assunto (nada do utilizador).
- A palavra para confirmar "Destruir perfil e dados" é 削除 / 删除 / हटाएं; em hindi aceita as duas
  formas do nasal (ँ e ं).
- Site em nove línguas (`ja.js`, `zh.js`, `hi.js`), seletor de línguas num menu com o nome de cada
  língua (sem bandeiras), letras do sistema próprias para japonês, chinês e hindi, e uma linha no
  rodapé das traduções para avisar de erros. O chinês tradicional do navegador fica em inglês.
- Nomes do exemplo "Como funciona": アナ/ルイ, 安娜/鲁伊 e, em hindi, अनु/रवि (em hindi "आना" quer dizer
  "vir" e "रुई" quer dizer "algodão").
- As traduções devem ser revistas (folha de revisão entregue ao Nuno).

### 22 de outubro de 2026: tempo de entrega só com o destinatário ligado (relay)

- O tempo de entrega (e o "entre telemóveis" do site) só conta mensagens para quem estava com a
  app aberta e ligada ao relay no momento em que a mensagem chegou. Antes contava também o tempo
  até a pessoa abrir a app (até 60 s), o que fazia o site mostrar mais de 20 s com poucos testers.
- O relay lembra-se, só em memória e durante 5 s, de quando acabou a última espera de cada conta
  (para não perder mensagens que chegam entre duas esperas). Teste: `server/tests/relay_entrega.rs`.

### 22 de outubro de 2026: correções das apps (0.9.23, Android e iPhone)

- Contactos: se a lista guardada não abrir, a app já não grava uma lista vazia por cima nem envia
  a cópia de recuperação vazia. Mostra um aviso: reabrir a app ou começar uma lista nova (a antiga
  fica guardada à parte, cifrada).
- Android: tocar numa notificação durante uma chamada já não corta a chamada. Fechar a app a meio
  de uma chamada avisa o outro lado.
- iPhone: um contacto bloqueado ou desconhecido já não põe o iPhone a tocar. A capa de privacidade
  passa a tapar também as fotos, os documentos e as outras janelas abertas.
- As 12 palavras de recuperação só aparecem (ou são geradas) depois do código da app ou do
  bloqueio do telemóvel.
- PNG e WebP enviados em qualidade original vão sem metadados (data, aparelho, localização), com a
  mesma imagem, a transparência e a animação.
- Um anexo que o relay recusa já não para os outros; ao fim de 5 tentativas fica como falhado. As
  tentativas ficam mais espaçadas.
- IDs em maiúsculas ("QT1:...") são aceites e guardados em minúsculas (também no núcleo). No
  iPhone, ler o QR de um ID passa a funcionar com o QR em maiúsculas.
- Bloqueio da app: passa a bloquear em segundo plano quando passa o tempo escolhido ("Imediatamente"
  bloqueia logo ao sair). Uma tentativa de código conta mesmo que a app seja fechada a meio.
- Contas novas: "Mostrar quando estou online" começa desligado. Quem já tinha conta fica como estava.
- Com o bloqueio da app ativo, o nome de quem liga nunca aparece no ecrã de chamada.
- Gravação de voz cancelada quando a app sai do ecrã (antes continuava e podia ser enviada sozinha).
- O texto de uma mensagem que não foi enviada volta à caixa de escrita.
- Mensagens e anexos já não são repetidos automaticamente quando a resposta do relay se perde.
- Mensagens para quem apagou a conta saem da fila de envio.
- Android: ficheiros recebidos abrem pelo tipo verdadeiro (conteúdo e extensão), nunca pelo que o
  remetente disse; aviso para ligar as chamadas em ecrã inteiro no Android 14+; vibração respeita o
  modo silêncio; aviso se o microfone falhar numa chamada; datas no formato da língua da app.
- iPhone: um Face ID de uma instalação antiga deixa de aparecer; microfone pedido ao atender;
  ficheiro de voz protegido desde o início; sem a exceção de redes locais na Release.
- Relay: um telemóvel só acorda pela conta mais recente que registou o seu token de notificações.
- Builds: Rust 1.88 declarado em todos os crates; compilação automática da app iPhone no GitHub.

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

### 22 de outubro de 2026: acesso por conta controlado pelo operador (apps 0.9.21, Android e iPhone)

- O acesso de cada conta às funções reservadas e o bloqueio passam a ser decididos só no relay, pela
  ferramenta do operador: bloqueio para todos, dar, tirar e lista.
- Com o bloqueio ligado, o relay recusa anexos e ofertas de chamada de contas sem acesso
  (`AdminPremiumEnforce`, `PREMIUM_REQUIRED`), mesmo de uma app alterada.
- As apps perguntam ao relay antes de cada ação reservada: tirar o acesso conta logo. Saíram o
  interruptor de teste e a simulação do acesso.

### 22 de outubro de 2026: acesso por conta (apps 0.9.20, Android e iPhone)

- Fazer chamadas, criar grupos e enviar fotos, GIFs, voz e ficheiros podem ficar reservados a contas
  com acesso dado pelo relay. Texto sempre aberto; receber tudo também.
- Bloqueio com interruptor de teste; com ele ligado, uma ação reservada sem acesso abre o ecrã
  próprio da app.
- O relay guarda até quando cada conta tem acesso (`PremiumStatus`, `AdminGrantPremium`,
  `AdminListPremium`). O operador dá ou tira o acesso (testadores) pela ferramenta `denuncias`.

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
