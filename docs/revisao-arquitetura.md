# Pedido de revisão de desenho e arquitetura (QT / SEALER, versão alpha)

Este documento é o ponto de partida para quem vai rever o desenho criptográfico e a
arquitetura do QT antes de o código crescer. Diz o que pedimos, o que não pedimos, por
onde ler e onde queremos uma opinião.

## 1. Objetivo

Validar a segurança e a estabilidade da **arquitetura básica** enquanto ainda é barato
mudar. Não é uma auditoria ao código completo nem um teste de intrusão: esses ficam para
a versão final, quando o protocolo estiver congelado.

**Pedimos:**

- Revisão do desenho criptográfico: PQXDH, Double Ratchet, Sealed Sender, autenticação no
  relay, anexos, grupos e presença, tal como estão especificados.
- Revisão da arquitetura do servidor: exposição à internet, isolamento, abusos e negação de
  serviço, ponto único de falha, cadeia de deploy.
- Uma opinião sobre as decisões em aberto da secção 5, ordenadas por quanto custaria
  mudá-las mais tarde.

**Não pedimos (nesta fase):**

- Revisão linha a linha do código Rust, Kotlin ou Swift.
- Testes de intrusão ao servidor de produção.
- Análise de canais laterais ou de tempo constante das dependências.

## 2. Estado do projeto

- Versão da app Android: 0.9.2 (alpha), testada em 3 telemóveis. A app iPhone está atrasada
  e fora do âmbito desta revisão.
- Um relay em produção (AWS eu-north-1, t3.small), usado só por testadores.
- Código aberto (AGPL, com licença comercial dupla), repositório `moral666/QT`, ramo
  `site-sealer`.
- Um único autor, assistido por IA. Sem revisão externa até hoje.

## 3. Arquitetura em resumo

```
App (Kotlin / Swift: só interface, nenhuma criptografia)
  └─ núcleo Rust por UniFFI: core (PQXDH, Double Ratchet, Sealed Sender),
     storage (SQLCipher, chave no Android Keystore / Secure Enclave), ffi
        │  wss://relay.sealer.pt (TLS 1.2+, certificado Let's Encrypt; DNS sem proxy)
        ▼
HAProxy (TLS, sem registos, limites por IP)  ──►  relay Rust em 127.0.0.1:9443
                                             Noise XX dentro do WebSocket, chave do relay
                                             fixa (pinning) na app
                                               ├─ Redis em 127.0.0.1 (filas, bundles; ACL mínima)
                                               ├─ anexos cifrados em disco (24 h)
                                               └─ FCM (Google) para a campainha genérica
```

- O relay vê: o destinatário de cada envelope, tamanhos, horas, o IP (no HAProxy) e o
  token FCM de cada utilizador. Não vê o conteúdo nem o remetente (Sealed Sender).
- Cada app aberta usa **duas ligações**: uma para pedidos e outra parada em `WaitMessages`
  (entrega imediata). O protocolo é um pedido, uma resposta, por ordem.
- Deploy: o servidor descarrega o código do GitHub e compila-o no próprio servidor
  (`deploy/aws/setup-ec2.sh`).

- Limites contra abusos (desde 19/10/2026): 100 ligações abertas e 100 novas a cada 10 s
  por IP no HAProxy; no relay, 2 000 ligações por autenticar e 8 por conta.

O diagrama `docs/arquitetura.svg` ainda não mostra as notificações (FCM). Vale o texto acima.

## 4. O que ler, por esta ordem

1. `docs/threat-model.md`: adversários, o que é protegido, com que teste, e o que não é.
2. `docs/protocol-spec.md`: secções 8 a 17 (sessão, bundle, envelope, protocolo do relay,
   payload, sessões cruzadas, anexos, grupos, presença, notificações).
3. `docs/known-limitations.md`: o que já sabemos que falta. **Não é preciso reportar o que
   já lá está**, mas queremos saber se a ordem de prioridade está certa.
4. `deploy/aws/setup-ec2.sh` e `deploy/systemd/qt-relay.service`: como o servidor é montado.
5. `docs/performance.md`: medições de capacidade.

Para correr os testes: ver a secção "Comandos" do `CLAUDE.md` (precisa de Redis local).

## 5. Decisões em aberto (onde queremos opinião)

Ordenadas por custo de mudar mais tarde. Cada uma tem a pergunta concreta.

| # | Tema | Pergunta |
|---|---|---|
| 1 | Endereço do relay: desde a app 0.9.2 é `relay.sealer.pt`, um registo DNS sem proxy (de propósito, para nenhum intermediário ver os IPs dos utilizadores). Sem proteção contra DDoS além da básica da AWS | Este compromisso (privacidade em vez de proteção contra DDoS) é aceitável? Há alternativas que não exponham os IPs a terceiros? |
| 2 | Uma só chave do relay fixa na app, sem forma de a trocar (já há cópia de segurança fora do servidor) | Que mecanismo de rotação recomendam (por exemplo, a app aceitar a chave atual e uma de reserva)? |
| 3 | HKDF do PQXDH sem *transcript binding* e com salt vazio | Vale a pena mudar já, enquanto só há testadores (as sessões atuais deixam de servir)? Que formato recomendam? |
| 4 | Sem one-time prekeys servidas pelo relay; signed pre-key e prekey ML-KEM sem rotação nem validade | É prioritário antes da versão 1.0? Que período de rotação? |
| 5 | Duas ligações por app, protocolo sequencial | Multiplexar pedidos numa só ligação (pedidos com identificador) traz algum risco de segurança que devamos considerar no desenho? |
| 6 | Já há limites de ligações por IP e por conta (secção 3), mas criar uma conta é gerar uma chave, sem custo, e não há limite de mensagens | Que defesa contra spam recomendam ao nível do desenho (prova de trabalho no registo, limite de mensagens, consentimento de contacto)? |
| 7 | Anexos: o relay vê que utilizador enviou e que utilizador descarregou cada ficheiro | Faz sentido descarregar os anexos por uma ligação sem autenticação, para não ligar quem fala com quem? |
| 8 | Tamanho das mensagens de texto sem enchimento; o campo `silent` diz ao relay que um envio não é uma mensagem | Quanto pesa esta fuga de metadados e que enchimento recomendam? |
| 9 | Deploy: o servidor compila código vindo do GitHub, como root | Devemos passar a compilar no CI e instalar só binários assinados? |
| 10 | Um só servidor e Redis sem réplica (sem segunda máquina, por custo) | Qual é o mínimo aceitável para uma versão pública sem aumentar o custo mensal? |

## 6. O que esperamos receber

- Um relatório com cada problema encontrado: o que está mal, o impacto e uma sugestão.
- A classificação de cada problema (crítico, alto, médio, baixo) e se é **estrutural**
  (mais caro de mudar com o tempo) ou pode esperar.
- Uma opinião sobre as 10 decisões da secção 5.

**Nenhuma alteração ao código é feita durante ou depois da revisão sem aprovação explícita
do Nuno Gonzalez, uma a uma.** Os revisores não precisam de propor código, só o problema,
o impacto e a direção da correção.

## 7. Contacto

Nuno Gonzalez, pelo mecanismo privado do GitHub (separador Security do repositório) ou
pelo contacto combinado diretamente.
