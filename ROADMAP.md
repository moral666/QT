# Roadmap

Este documento existe para responder a uma pergunta simples: **o que
falta, e por que ordem faz sentido fazer?** A tabela de estado no
`README.md` mostra o que já está feito; este ficheiro mostra o que vem a
seguir.

## Feito

- **Fundação técnica:** núcleo E2EE (X3DH, PQXDH, Double Ratchet, Sealed
  Sender), transporte (Noise sobre WebSocket), relay com persistência real
  (Redis), armazenamento local cifrado (SQLCipher), cliente de terminal e
  bindings FFI para Kotlin, Swift e Python.
- **Apps Android e iPhone (SEALER)** com a conversa completa: lista de
  conversas, chat, histórico guardado no SQLCipher, estados *enviada /
  entregue / vista*, apagar conversa e contacto, verificação de segurança
  (safety numbers), adicionar contactos por código QR, fotos, GIFs, voz e
  ficheiros até 25 MB, reações, convites e grupos até 10 pessoas (app 0.9.3).
- **Notificações push com a app fechada** (aviso genérico "Tens mensagens novas";
  desde a 0.9.6 a app decifra em segundo plano, no telemóvel, para as chamadas tocarem).
- **Relay em `relay.sealerapp.com`** (HAProxy com TLS), medido com 8 000 ligações
  em simultâneo sem falhas; desde a 0.9.45 cada app usa uma só ligação.
- **Relay acessível pela internet** (AWS EC2), atrás de TLS (`wss://`), com
  pinning da chave Noise nas apps e o Redis só em `127.0.0.1` com ACL.
- **Chave da base de dados vinda do Android Keystore / Secure Enclave do iPhone**; o
  `messenger` do terminal pede a passphrase escondida, nunca na linha de
  comandos.
- **Chave estática Noise do relay persistida** entre reinícios (pinning real).
- **Registo de bundles autenticado:** o relay só aceita o bundle da
  identidade que se autenticou na ligação.
- **Bloqueio da app** (código próprio e biometria, Face ID no iPhone).
- **Teste de campo** com cinco aparelhos em três plataformas
  (`docs/field-tests.md`).
- **Chamadas de voz no Android** (19/10/2026) **e no iPhone** (testadas a 20/10/2026): sinalização dentro do Double Ratchet,
  relay das chamadas em produção (UDP 443, salas anónimas, reserva por WebSocket),
  motor de áudio (Opus 1.6 com DRED e PLC neural, jitter buffer adaptativo), com a app
  aberta (0.9.4) e a tocar com a app fechada (0.9.6), com toque próprio (0.9.8).
- **Foto de perfil** só para contactos, cifrada (0.9.9), e **app em seis línguas**
  (PT, EN, FR, ES, DE, IT, 0.9.10); japonês, chinês simplificado e hindi a 22/10/2026; árabe (da direita para a esquerda) a 23/10/2026.
- **Novo design** (0.9.11): cabeçalho com o fundo curvo, ecrã de Definições e ecrã Novo
  contacto; foto de perfil com pré-visualização e "Aplicar".
- **Desempenho** (0.9.12): lista de conversas que não fica mais lenta com o histórico,
  fotos de perfil com menos memória, chamadas sem gastar CPU quando o microfone falha e
  APK mais pequena.
- **Medalhas** (0.9.14 e 0.9.16): aro e medalha na foto (Founder, Co-founder, Dev, Tester,
  Contributor), com certificado Ed25519 enviado cifrado aos contactos; o servidor não guarda nada.
- **Revisão de segurança do núcleo** (0.9.17): o relay só aceita escritas de contas registadas,
  chaves intermédias zeradas na memória e fuzzing com `cargo-fuzz`.
- **Denúncias e banimentos** (0.9.18): denunciar um contacto com o motivo e até 20 mensagens,
  cifradas para a equipa da SEALER; o relay guarda-as sem as ler e sem guardar quem denunciou.
- **Chave de recuperação** (0.9.19): 12 palavras recuperam o ID, os contactos, os bloqueados
  e a medalha num telemóvel novo (sem histórico nem grupos).
- **Revisão das apps** (0.9.22): espera depois de códigos errados que não se salta mudando a
  hora, apps recentes tapadas no Android 7 a 12 e medalha logo depois de recuperar a conta.
- **Pedidos dos testadores** (0.9.29 a 0.9.31): toque de quem liga desde o início, altifalante
  antes de atender, chamada minimizada com a app usável, copiar as 12 palavras, links que só abrem
  depois de confirmar o site (sem pré-visualização, por privacidade), medalhas e fotos que voltam
  a quem recuperou a conta, menu Idioma que desliza.
- **Chamadas do Android no sistema de chamadas do telemóvel** (0.9.52, Core-Telecom): auricular
  Bluetooth, carro e chamadas GSM ao mesmo tempo, como o CallKit no iPhone, sem nada no registo de
  chamadas do telemóvel.
- **Comunidade** (0.9.52, Android e iPhone): as pessoas com medalha, só com foto e medalha (sem
  nome nem ID), atualizada uma vez por semana no Sealer Control, cifrada nos dois relays e
  descarregada só quando se abre o ecrã.
- **Vídeo só pela ligação direta** (0.9.53, Android e iPhone): a imagem das videochamadas vai de
  telemóvel para telemóvel entre contactos mútuos e nunca passa pelo servidor (até 1080p no Wi-Fi).
  Sem ligação direta, a chamada continua só com voz.
- **Qualidade do vídeo** (0.9.61): sem teto fixo no Wi-Fi (até 4K, conforme a ligação e os
  telemóveis), até 720p com dados móveis; a voz manda (o vídeo desce antes de a voz chegar
  atrasada); o atraso a mais da voz desce nos silêncios.
- **Chamadas de grupo** (0.9.62 a 0.9.67, Android; iPhone por compilar): até 5 pessoas, voz e
  vídeo, cifradas ponta a ponta; a minha câmara em cima e os outros na grelha; nome escolhido
  para cada chamada; entrar depois de recusar; diagnóstico como nas chamadas a dois.

## A seguir: tornar isto usável no dia a dia

- [x] **Chamadas de voz na app Android com a app aberta** (fase 4a, app 0.9.4).
- [x] **Chamadas a tocar com a app fechada** (fase 4b, app 0.9.6).
- [x] **App iPhone igual à Android** (0.9.13): foto de perfil, seis línguas, novo design e
  bloqueio da app (código e Face ID).
- [x] **Chamadas no iPhone** (CallKit), entre iPhones e entre Android e iPhone. Testado a
  20/10/2026 com Android 0.9.16 e iPhone 0.9.15, junto com contactos, QR, mensagens de texto
  e voz, GIFs, ficheiros, bloquear e apagar conversas: sem erros.
- [x] **No Android, a integração com o sistema de chamadas** (auricular Bluetooth, carro,
  chamada normal ao mesmo tempo), app 0.9.52. Falta o teste nos telemóveis.
- [ ] **Infraestrutura de produção:** backups e alertas do servidor.
- [ ] **Distribuição:** Google Play e App Store (TestFlight).

## Robustez e confiança

- [x] **Testes com entradas inválidas e forjadas:** lixo nas entradas de rede
  e de disco, mensagens e cabeçalhos forjados, limites de tamanho, e testes
  por propriedades do Double Ratchet (200 sequências aleatórias de entrega em
  cada corrida). Ver `docs/threat-model.md`, secção 3.
- [x] **Fuzzing guiado por cobertura** (`cargo-fuzz`, 5 alvos em `fuzz/`, 0.9.17): a
  primeira volta não encontrou erros. Falta correr durante horas, com regularidade.
- [x] **Limites no relay por utilizador** (0.9.3): ligações por IP e por conta,
  envios e pedidos de bundle por minuto, prova de trabalho ao criar conta.
- [x] **One-time prekeys servidas pelo relay**, uma por pedido (0.9.3).
- [x] **Validade e rotação** da signed pre-key e da prekey ML-KEM (0.9.3, 7 dias).
- [x] **PQXDH v2** com transcrição no HKDF (0.9.3).
- [ ] **Reinício de sessão completo:** falta o caso em que quem apagou o
  contacto recebe a primeira mensagem (ver `docs/known-limitations.md`, 10).
- [x] **Enchimento do tamanho das mensagens de texto** (0.9.3: 4 tamanhos fixos).
- [ ] **Jitter de timing**, para dificultar a análise de tráfego por quem observe o relay.

## Fase de escala (mais distante, mas desenhada desde o início)

- [x] **Federação** (29/10/2026): federação fechada com dois relays, `estocolmo` e
  `espanha`, que entregam entre si mensagens, bundles, anexos, cópias de recuperação e
  chamadas (`docs/protocol-spec.md`, secção 25). O desenho aceita mais relays.
- [ ] **Federação, a seguir:** mais relays na Europa; mudar uma conta de relay; continuar
  a receber quando o relay de casa da conta está em baixo (hoje fica à espera que volte)
- [ ] **Grupos maiores:** os grupos atuais (até 10 pessoas) enviam uma cópia
  cifrada a cada membro; para grupos grandes faz sentido Sender Keys ou MLS

## Fora do roadmap (por agora, deliberadamente)

Anonimato de rede completo (Tor/mixnet), deteção automática de abuso (há
denúncias feitas pelas pessoas, desde a 0.9.18) e proteção contra coerção
física não estão planeados para o curto/médio prazo. O que
o QT não protege está em `docs/threat-model.md`, secção 4.

## Como isto se mantém atualizado

Este ficheiro é editado manualmente à medida que o projeto avança; não
há automação a sincronizá-lo com o código. Se encontrares uma
inconsistência entre isto e o estado real do repositório, é um bug de
documentação tão válido para reportar como um bug de código.
