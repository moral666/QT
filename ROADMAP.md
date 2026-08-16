# Roadmap

Este documento existe para responder a uma pergunta simples: **o que
falta, e por que ordem faz sentido fazer?** A tabela de estado no
`README.md` mostra o que já está feito; este ficheiro mostra o que vem a
seguir.

## Fase atual: fundação técnica (concluída)

Tudo o que está listado como "✅ Testado" na tabela do `README.md`:
núcleo E2EE (X3DH, PQXDH, Double Ratchet, Sealed Sender), transporte
(Noise sobre WebSocket), servidor com persistência real (Redis),
armazenamento local encriptado (SQLCipher), cliente de terminal, e
bindings FFI para Kotlin/Swift/Python, testados também num Android real.

## Próxima fase: tornar isto usável de verdade

- [ ] **Ecrã de conversa completo na app Android** — hoje só existe um
  botão de demonstração; falta a experiência real (lista de contactos,
  conversa, notificações)
- [ ] **Servidor acessível pela internet**, não só em rede local — para
  duas pessoas em redes diferentes conseguirem falar
- [ ] **TLS (`wss://`)** na camada de transporte, por cima do Noise já
  existente — defesa em profundidade adicional
- [ ] **Passphrase da base de dados vinda do Keystore/Secure Enclave**,
  em vez de argumento de linha de comandos

## Fase seguinte: robustez e confiança

- [x] **Testes de robustez contra bytes malformados** — encontrado e
  corrigido um crash real (`bytes[0]` sem verificação de tamanho em
  `cli/src/bin/messenger.rs`); 40.000+ tentativas aleatórias sem pânico
  contra as funções mais expostas do núcleo (ver `docs/threat-model.md`)
- [ ] **Fuzzing guiado por cobertura, corrido a sério** — os alvos já
  existem em `core/fuzz/` e compilam, mas nunca correram durante horas
  numa máquina com Rust nightly (só testes de bytes aleatórios mais
  simples correram até agora) — ver `core/fuzz/README.md`
- [ ] **Auditoria de segurança externa** — nenhuma alegação de segurança
  deveria ser levada a sério sem isto; é a prioridade mais alta desta
  fase
- [ ] **Persistência da chave estática Noise do servidor entre
  reinícios**, para "pinning" real do lado do cliente
- [ ] **One-time pre-key consumida uma única vez de verdade** — o
  cliente de terminal atual reutiliza sempre a mesma, simplificação
  documentada que precisa de correção antes de produção
- [ ] **Autenticação no registo de bundles** — impedir que alguém
  publique um bundle a fingir ser outra identidade (ver
  `docs/threat-model.md`)
- [ ] **Padding de tamanho e jitter de timing** — mitigar análise de
  tráfego por um adversário que observe o servidor (ver limitações no
  `docs/threat-model.md`)

## Fase de escala (mais distante, mas desenhada desde o início)

- [ ] **Federação** — vários servidores independentes, em vez de um
  único, para que nenhum governo ou operador consiga desligar a rede
  inteira desligando só um nó
- [ ] **Suporte a grupos** (conversas com mais de duas pessoas) — exige
  uma extensão ao protocolo atual (ex.: Sender Keys ou MLS)
- [ ] **Apps iOS** — o `ffi/` já gera bindings Swift, mas nunca foram
  testados num dispositivo Apple real (só Android, por limitação de
  ambiente de desenvolvimento)

## Fora do roadmap (por agora, deliberadamente)

Ver `docs/threat-model.md`, secção "Fora de escopo" — anonimato de rede
completo (Tor/mixnet), deteção de abuso, e proteção contra coerção física
não estão planeados para o curto/médio prazo.

## Como isto se mantém atualizado

Este ficheiro é editado manualmente à medida que o projeto avança — não
há automação a sincronizá-lo com o código. Se encontrares uma
inconsistência entre isto e o estado real do repositório, é um bug de
documentação tão válido para reportar como um bug de código.
