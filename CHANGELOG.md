# Changelog

Formato inspirado em [Keep a Changelog](https://keepachangelog.com/).
Datas aproximadas: este projeto ainda não segue versionamento semântico
formal (não há releases numeradas ainda, ver `ROADMAP.md`).

## [Não lançado]

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
