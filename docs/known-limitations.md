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
4. **Ratchet sem `pn` no cabeçalho:** mensagens atrasadas da cadeia DH anterior
   perdem-se quando chega primeiro uma da cadeia nova (sessão intacta).
5. **Chaves de mensagens puladas** ficam no estado até serem usadas; `MAX_SKIP`=1000.
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
17. **Redis:** sem TLS/ACL configurados por este projeto; as filas são
    ciphertext, mas o Redis exposto permite apagar/atrasar mensagens.
18. **Mensagens de texto ≤ 10 000 bytes** (JSON dentro de um frame Noise de 64 KiB).
    Imagens e GIFs ate 25 MiB seguem por outro caminho (ver 36).

## Android / app
19. **`build-android.sh` não foi executado** pelo autor desta fase (sem
    NDK/rustup no ambiente). Testado só no host com as mesmas features.
20. **A chave do storage passa pela memória da JVM** no arranque.
21. **O histórico em claro para a UI não é gerido pelo Rust**: tem de ser
    protegido pela app (risco se ignorado).
22. **Lógica de reconnect/ack do Kotlin ainda por escrever**; a referência
    testada está em Rust (`RelayClient`).
23. **Sem backup/restauro** entre dispositivos (por desenho: `allowBackup=false`).
24. **Dispositivo comprometido, root, malware, depuração** fora de âmbito.

## Código
25. **CLI e demos** (`cli/`) usam `expect`/`unwrap` e a passphrase por argumento
    de linha de comandos: são ferramentas de demonstração, não de produção.
26. **`legacy-api`** (FFI antiga) expõe chaves privadas; só para testes.
27. `cargo audit`/`cargo deny` não foram corridos; dependências não revistas.
28. Os testes de relay precisam de um Redis local em `127.0.0.1:6379`.

## App Android de referência (`QTApp/`)
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
33. **Poll de 3 s** em vez de *push*; sem notificações. Enquanto o Android não termina a app, ela
    continua a perguntar ao relay em segundo plano, o que gasta bateria e dados.
34. Sem ABI `x86_64` com o NDK r26d (o OpenSSL embebido não compila o assembly x86 de SM3/SM4);
    testar em telemóvel arm64, ou compilar com um NDK mais recente.
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
    descarga, sem vídeos nem outros ficheiros, e a app iPhone só recebe GIFs do teclado colando-os.
    O tamanho e as dimensões da imagem vão dentro da mensagem cifrada; a localização da foto
    é removida ao recomprimir (as originais PNG/WebP/GIF vão como estão).
