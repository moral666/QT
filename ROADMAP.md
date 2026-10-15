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
  (safety numbers), adicionar contactos por código QR, imagens e GIFs até
  25 MB.
- **Relay acessível pela internet** (AWS EC2), atrás de TLS (`wss://`), com
  pinning da chave Noise nas apps e o Redis só em `127.0.0.1` com ACL.
- **Chave da base de dados vinda do Android Keystore / Secure Enclave**; o
  `messenger` do terminal pede a passphrase escondida, nunca na linha de
  comandos.
- **Chave estática Noise do relay persistida** entre reinícios (pinning real).
- **Registo de bundles autenticado:** o relay só aceita o bundle da
  identidade que se autenticou na ligação.
- **Bloqueio da app no Android** (código próprio e biometria).
- **Teste de campo** com cinco aparelhos em três plataformas
  (`docs/field-tests.md`).

## A seguir: tornar isto usável no dia a dia

- [ ] **Notificações push** (Android e iPhone; no iPhone exige a conta paga da
  Apple). Hoje as apps perguntam ao relay de 3 em 3 segundos enquanto estão
  abertas, e o Android continua a perguntar em segundo plano até fechar a
  app, o que gasta bateria e dados.
- [ ] **Bloqueio da app no iPhone**, igual ao do Android.
- [ ] **Confirmar as imagens e os GIFs entre telemóveis** através do relay da
  AWS, e compilar no Xcode as novidades da app iPhone.
- [ ] **Infraestrutura de produção:** domínio próprio, backups e alertas do
  servidor.
- [ ] **Distribuição:** Play Store e TestFlight.
- [ ] **Teste de carga no relay da AWS** (até agora só medido numa máquina
  local, ver `docs/performance.md`).

## Robustez e confiança

- [x] **Testes com entradas inválidas e forjadas:** lixo nas entradas de rede
  e de disco, mensagens e cabeçalhos forjados, limites de tamanho, e testes
  por propriedades do Double Ratchet (200 sequências aleatórias de entrega em
  cada corrida). Ver `docs/threat-model.md`, secção 3.
- [ ] **Fuzzing guiado por cobertura** (`cargo-fuzz`), corrido durante horas:
  ainda não existe no repositório.
- [ ] **Auditoria de segurança externa**: nenhuma alegação de segurança
  deveria ser levada a sério sem isto; é a prioridade mais alta desta fase.
- [ ] **Limites no relay por utilizador** (rate limiting, anti-spam, limite
  de bytes por fila). Hoje há só o limite de 1000 mensagens por fila e a
  quota diária das imagens.
- [ ] **One-time prekeys servidas pelo relay**, uma por pedido. Hoje o relay
  entrega o mesmo bundle a todos, sem one-time prekey (o PQXDH continua
  válido, mas perde essa proteção extra).
- [ ] **Validade e rotação** da signed pre-key e da prekey ML-KEM.
- [ ] **Reinício de sessão completo:** falta o caso em que quem apagou o
  contacto recebe a primeira mensagem (ver `docs/known-limitations.md`, 10).
- [ ] **Padding de tamanho das mensagens de texto e jitter de timing**, para
  dificultar a análise de tráfego por quem observe o relay (as imagens já vão
  com o tamanho arredondado).

## Fase de escala (mais distante, mas desenhada desde o início)

- [ ] **Federação**: vários servidores independentes, em vez de um
  único, para que a falha ou a manutenção de um nó não pare a rede
  inteira
- [ ] **Suporte a grupos** (conversas com mais de duas pessoas): exige
  uma extensão ao protocolo atual (ex.: Sender Keys ou MLS)

## Fora do roadmap (por agora, deliberadamente)

Anonimato de rede completo (Tor/mixnet), deteção de abuso e proteção
contra coerção física não estão planeados para o curto/médio prazo. O que
o QT não protege está em `docs/threat-model.md`, secção 4.

## Como isto se mantém atualizado

Este ficheiro é editado manualmente à medida que o projeto avança; não
há automação a sincronizá-lo com o código. Se encontrares uma
inconsistência entre isto e o estado real do repositório, é um bug de
documentação tão válido para reportar como um bug de código.
