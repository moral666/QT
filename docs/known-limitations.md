# Limitações conhecidas (v0.2)

Ordenadas por impacto. O detalhe do modelo de adversário está em
`threat-model.md`. **Nenhum item aqui foi resolvido por "o teste passa".**

## Segurança / criptografia
1. **Sem auditoria externa.** Todo o código (incl. a integração `ml-kem 0.2.3`) é
   de um único autor, assistido por IA. Não usar para proteger pessoas em risco.
2. **Autenticação e Sealed Sender são clássicos** (Ed25519/X25519). Só o segredo
   inicial é híbrido pós-quântico. O transporte Noise é clássico.
3. **HKDF do PQXDH sem *transcript binding*** e com salt vazio (mantido por
   compatibilidade com o formato v0.1). Não é uma falha conhecida, mas é mais
   fraco que o recomendado para novos desenhos.
4. ~~**Ratchet sem `pn` no cabeçalho**~~ (resolvido): o cabeçalho leva `PN` e as mensagens
   atrasadas da cadeia DH anterior decifram-se com as chaves puladas
   (`fora_de_ordem_entre_cadeias_dh_recupera_com_pn`).
5. **Chaves de mensagens puladas** ficam no estado até serem usadas (quem roubar o estado nesse
   momento obtém-nas); no máximo 1000 por cadeia e 2000 no total (as mais antigas são descartadas).
6. **Sem one-time prekeys servidas pelo relay** (bundle único para todos).
7. **Bundles sem validade e sem rotação** da signed pre-key / prekey ML-KEM;
   um relay pode servir um bundle antigo.
8. **Rollback do ficheiro SQLCipher** não é detetado (sem contador monotónico).
9. **Zeroização incompleta** de chaves nas dependências e na JVM.
10. **Reinício de sessão incompleto.** Já existem *safety numbers* e a adoção
    automática de uma sessão nova autêntica (sessões cruzadas, contacto que
    apagou e voltou a escrever). Falta o caso em que quem apagou o contacto
    recebe primeiro: hoje essas mensagens são descartadas (ver
    `field-tests.md`). Uma reinstalação da app gera uma identidade nova.
11. **Sem verificação de tempo constante / canais laterais** das dependências.

## Relay / rede
12. **Sem rate limiting, anti-spam nem consentimento de contacto**; fila ≤ 1000.
13. **Metadados visíveis ao relay:** destinatário, tamanho, instante, IP.
14. **Sem federação:** ponto único de falha e de pressão legal.
15. **`ws://` nos testes.** Produção: `wss://` + pinning da chave Noise.
16. **A chave estática Noise do relay está em ficheiro** (0600), sem HSM.
17. **Redis:** no deploy (`deploy/redis/qt-redis.conf`) escuta só em `127.0.0.1`, com o utilizador
    `default` desligado e um utilizador com a lista mínima de comandos; sem TLS (não sai da
    máquina). As filas são ciphertext, mas quem chegar ao Redis pode apagar ou atrasar mensagens.
18. **Mensagens de texto ≤ 10 000 bytes** (JSON dentro de um frame Noise de 64 KiB).
    Imagens e GIFs ate 25 MiB seguem por outro caminho (ver 36).

## Android / app
19. ~~**`build-android.sh` não foi executado**~~ (resolvido): é o script usado para as APKs
    testadas em telemóveis reais.
20. **A chave do storage passa pela memória da JVM** no arranque.
21. ~~**O histórico em claro para a UI não é gerido pelo Rust**~~ (resolvido): o histórico fica
    no SQLCipher do Rust e as imagens ficam cifradas no disco. Em claro, as apps têm em memória o
    que mostram e, durante o envio de uma imagem, um ficheiro temporário com a imagem já
    comprimida, apagado logo a seguir. Isto depende de as apps não escreverem mais nada em claro.
22. ~~**Lógica de reconnect/ack do Kotlin ainda por escrever**~~ (resolvido): escrita em
    `RelayLink.kt` / `RelayLink.swift` e no `MainViewModel` / `AppModel`, sem testes automáticos
    de interface; a referência testada continua em Rust (`RelayClient`).
23. **Sem backup/restauro** entre dispositivos (por desenho: `allowBackup=false`).
24. **Dispositivo comprometido, root, malware, depuração** fora de âmbito.

## Código
25. **Demos** (`messenger_demo`) usam `expect`/`unwrap`: são ferramentas de demonstração, não
    de produção. O `messenger` pede a passphrase escondida no terminal (nunca por argumento).
26. **`legacy-api`** (FFI antiga) expõe chaves privadas; só para testes.
27. `cargo audit` corre no CI (vulnerabilidades conhecidas); `cargo deny` não, e as dependências
    não foram revistas à mão.
28. Os testes de relay precisam de um Redis local em `127.0.0.1:6379`.

## Apps Android e iPhone (`android/`, `ios/`)
29. ~~**Sessões cruzadas**~~ (resolvido): quando as duas pessoas enviam a primeira mensagem ao mesmo
    tempo, um desempate determinístico fica com uma das sessões e as mensagens cifradas com a outra
    são reenviadas (teste `sessoes_cruzadas_nao_perdem_mensagens`; no teste prolongado de 30 minutos,
    nenhuma mensagem perdida).
30. **Para responder a quem nos escreveu primeiro** é preciso o QT Public ID dessa pessoa (o Sealed
    Sender só revela a chave de identidade, não a caixa de correio no relay).
31. ~~**Histórico só em memória**~~ (resolvido): o histórico fica no SQLCipher, na mesma transação
    que o avanço do ratchet.
32. ~~**Mensagens por enviar só em memória**~~ (resolvido): a outbox fica no SQLCipher e sobrevive
    ao reinício da app.
33. ~~**Poll de 3 s** em vez de *push*~~ (resolvido com a app aberta, 16/10/2026): entrega
    imediata com `WaitMessages` (o relay responde no instante em que a mensagem chega; ver
    `docs/protocol-spec.md`). Continua sem notificações com a app fechada (precisa de FCM/APNs;
    o interruptor "Notificações" do menu só decide se a app envia o aviso de "vista"). Android
    (desde a 0.7.0): 30 s depois de a app sair do ecrã, fecha as ligações ao relay (antes
    ficavam abertas, com ping e pedido de espera de 25 em 25 s, a gastar bateria sem mostrar
    nada); ao voltar, religa e vai buscar o que chegou. Até lá, quem envia vê só ✓.
34. Sem ABI `x86_64` com o NDK r26d (o OpenSSL embebido não compila o assembly x86 de SM3/SM4).
    Com o NDK 30 (30.0.16248370) compila: `ABIS="arm64-v8a armeabi-v7a x86_64"`. Sem a
    x86_64, a app fecha logo no emulador do Android Studio (não encontra `libqt_ffi.so`).
35. **Bloqueio da app (Android):** o código só tem números (mínimo 6). Fora do telemóvel não serve
    para nada, porque a chave cifrada também depende do Keystore do aparelho. Mas num telemóvel com
    root, quem conseguir correr código como a app pode tentar códigos através desse Keystore: cada
    tentativa custa cerca de 0,5 a 1 s (Argon2id, 64 MiB), por isso um código de 6 dígitos aguenta
    dias, não anos. Códigos mais longos protegem mais. A espera crescente depois de 5 erros só trava
    tentativas pela interface. Com o desbloqueio por impressão digital ativo, quem souber o PIN do
    telemóvel também abre a app (é uma escolha do utilizador). Ainda não existe na app iPhone.
36. **Imagens e GIFs (anexos).** O conteúdo e a chave são cifrados ponta a ponta, mas o relay vê:
    o tamanho arredondado (Padme, revela cerca de metade dos bits do tamanho), a hora do envio e
    da descarga e, enquanto a ligação está aberta, que utilizador enviou e que utilizador
    descarregou cada ficheiro. Correlacionar isto liga quem fala com quem, que o Sealed Sender
    esconde nas mensagens de texto. O relay não guarda esta ligação (nem nos registos), mas um
    relay malicioso podia guardá-la. Quem tiver o `blob_id` consegue ler (cifrado) e apagar o
    ficheiro. Um anexo não descarregado em 24 horas perde-se. Ainda sem miniatura antes da
    descarga, sem vídeos, e a app iPhone só recebe GIFs do teclado colando-os.
    O tamanho e as dimensões da imagem vão dentro da mensagem cifrada; a localização da foto
    é removida ao recomprimir (as originais PNG/WebP/GIF vão como estão).
37. **Grupos (desde a 0.8.0).** Texto, reações e, desde a 0.9.0, fotos, GIFs, voz e ficheiros. Sem
    recibos de entregue e vista. Os anexos de grupo ficam no relay (cifrados) até 24 horas, mesmo
    depois de todos os descarregarem: quem recebe não os apaga, porque os outros membros ainda
    podem estar a descarregar. Para escrever no grupo é preciso sessão com cada membro: quem
    ainda não publicou as chaves no relay fica de fora dessa mensagem (não volta a recebê-la
    depois). Cada mensagem sai uma vez por membro (até 9 envelopes), por isso gasta mais
    dados do que uma conversa a dois. Os membros veem os Public IDs uns dos outros (estão na
    info do grupo, cifrada).
38. **App iPhone desatualizada:** ainda não tem convites, reações, voz, grupos nem presença (o código Swift
    tem de ser atualizado aos tipos novos do núcleo Rust antes da próxima build iOS).
39. **Online / offline.** Só com a app aberta (sem notificações, a app fechada não diz nada).
    Se a app morrer ou o telemóvel ficar sem rede, os contactos continuam a ver "online" até 5
    min. Bloquear a app (código) não manda "offline": passa a offline ao fim desses 5 min. O
    relay não sabe o que é, mas vê que, ao abrir a app, sai um envelope para cada contacto
    (como uma rajada de mensagens pequenas). Mostra-se só "online": não há "visto por último".
40. **Ficheiros (desde a 0.9.0).** PDF, documentos e outros ficheiros até 25 MB vão tal como
    estão (sem compressão nem limpeza de metadados, ao contrário das fotos). O nome e o tipo
    vão dentro da mensagem cifrada. A app nunca mostra um ficheiro: abre-o noutra app, que o
    lê da memória do SEALER (sem cópia decifrada em disco, até 10 minutos ou até a app
    bloquear), ou guarda-o onde o utilizador escolher. Em ambos os casos, a partir daí, a
    cópia está fora do SEALER e a outra app pode guardá-la. As versões 0.8 e anteriores
    mostram "atualiza a app" no lugar de um ficheiro.
