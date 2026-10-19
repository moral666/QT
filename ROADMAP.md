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
- **Relay em `relay.sealer.pt`** (HAProxy com TLS), medido com 8 000 ligações
  em simultâneo sem falhas.
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
- **Chamadas de voz no Android** (19/10/2026): sinalização dentro do Double Ratchet,
  relay das chamadas em produção (UDP 443, salas anónimas, reserva por WebSocket),
  motor de áudio (Opus 1.6 com DRED e PLC neural, jitter buffer adaptativo), com a app
  aberta (0.9.4) e a tocar com a app fechada (0.9.6), com toque próprio (0.9.8).
- **Foto de perfil** só para contactos, cifrada (0.9.9), e **app em seis línguas**
  (PT, EN, FR, ES, DE, IT, 0.9.10). Por agora só no Android.
- **Novo design** (0.9.11): cabeçalho com o fundo curvo, ecrã de Definições e ecrã Novo
  contacto; foto de perfil com pré-visualização e "Aplicar". Por agora só no Android.
- **Desempenho** (0.9.12): lista de conversas que não fica mais lenta com o histórico,
  fotos de perfil com menos memória, chamadas sem gastar CPU quando o microfone falha e
  APK mais pequena.

## A seguir: tornar isto usável no dia a dia

- [x] **Chamadas de voz na app Android com a app aberta** (fase 4a, app 0.9.4).
- [x] **Chamadas a tocar com a app fechada** (fase 4b, app 0.9.6).
- [ ] **Chamadas no iPhone**, e no Android a integração com o sistema de chamadas
  (auricular Bluetooth, carro, chamada normal ao mesmo tempo).
- [ ] **Foto de perfil e seis línguas no iPhone.**
- [ ] **Bloqueio da app no iPhone**, igual ao do Android.
- [ ] **Infraestrutura de produção:** backups e alertas do servidor.
- [ ] **Distribuição:** Google Play e App Store (TestFlight).

## Robustez e confiança

- [x] **Testes com entradas inválidas e forjadas:** lixo nas entradas de rede
  e de disco, mensagens e cabeçalhos forjados, limites de tamanho, e testes
  por propriedades do Double Ratchet (200 sequências aleatórias de entrega em
  cada corrida). Ver `docs/threat-model.md`, secção 3.
- [ ] **Fuzzing guiado por cobertura** (`cargo-fuzz`), corrido durante horas:
  ainda não existe no repositório.
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

- [ ] **Federação**: vários servidores independentes, em vez de um
  único, para que a falha ou a manutenção de um nó não pare a rede
  inteira
- [ ] **Grupos maiores:** os grupos atuais (até 10 pessoas) enviam uma cópia
  cifrada a cada membro; para grupos grandes faz sentido Sender Keys ou MLS

## Fora do roadmap (por agora, deliberadamente)

Anonimato de rede completo (Tor/mixnet), deteção de abuso e proteção
contra coerção física não estão planeados para o curto/médio prazo. O que
o QT não protege está em `docs/threat-model.md`, secção 4.

## Como isto se mantém atualizado

Este ficheiro é editado manualmente à medida que o projeto avança; não
há automação a sincronizá-lo com o código. Se encontrares uma
inconsistência entre isto e o estado real do repositório, é um bug de
documentação tão válido para reportar como um bug de código.
