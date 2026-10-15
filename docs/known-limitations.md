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
18. **Mensagens ≤ 10 000 bytes** (JSON dentro de um frame Noise de 64 KiB);
    sem anexos nem fragmentação.

## Android / app
19. **`build-android.sh` não foi executado** pelo autor desta fase (sem
    NDK/rustup no ambiente). Testado só no host com as mesmas features.
20. **A chave do storage passa pela memória da JVM** no arranque.
21. **O histórico em claro para a UI não é gerido pelo Rust** — tem de ser
    protegido pela app (risco se ignorado).
22. **Lógica de reconnect/ack do Kotlin ainda por escrever**; a referência
    testada está em Rust (`RelayClient`).
23. **Sem backup/restauro** entre dispositivos (por desenho: `allowBackup=false`).
24. **Dispositivo comprometido, root, malware, depuração** fora de âmbito.

## Código
25. **CLI e demos** (`cli/`) usam `expect`/`unwrap` e a passphrase por argumento
    de linha de comandos — são ferramentas de demonstração, não de produção.
26. **`legacy-api`** (FFI antiga) expõe chaves privadas; só para testes.
27. `cargo audit`/`cargo deny` não foram corridos; dependências não revistas.
28. Os testes de relay precisam de um Redis local em `127.0.0.1:6379`.

## App Android de referência (`QTApp/`)
29. **Sessões cruzadas:** se duas pessoas enviarem a *primeira* mensagem uma à outra ao mesmo tempo,
    cada lado inicia a sua sessão PQXDH e as mensagens iniciais ficam indecifráveis (o core não
    resolve sessões simultâneas). A app mitiga: "adicionar contacto" só guarda o Public ID e a
    sessão só é criada ao enviar a 1.ª mensagem; mesmo assim o caso simultâneo continua por tratar
    (precisa de uma regra de desempate ou de um *session reset*).
30. **Para responder a quem nos escreveu primeiro** é preciso o QT Public ID dessa pessoa (o Sealed
    Sender só revela a chave de identidade, não a caixa de correio no relay).
31. **Histórico só em memória** na app de referência (o Rust nunca guarda plaintext). Persistir
    histórico exige uma base cifrada própria da app.
32. **Mensagens por enviar** (*outbox*) só existem em memória: se a app morrer entre cifrar e enviar,
    a mensagem perde-se (o ratchet já avançou; o destinatário tolera a falha na numeração).
33. **Poll de 3 s** em vez de *push*; sem serviço em segundo plano nem notificações.
34. Sem ABI `x86_64` com o NDK r26d (o OpenSSL embebido não compila o assembly x86 de SM3/SM4);
    testar em telemóvel arm64, ou compilar com um NDK mais recente.
