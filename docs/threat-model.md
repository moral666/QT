# Modelo de Ameaças

Este documento explica, de forma explícita, contra quem e contra o quê o
QT protege — e, tão importante quanto isso, contra quem e contra o quê
**não** protege. Um projeto de segurança sem isto é um projeto que pede
para ser avaliado por fé, não por engenharia.

## Ativos a proteger

1. **Conteúdo das mensagens** — o texto real trocado entre duas pessoas.
2. **Identidade do remetente** — quem enviou uma mensagem específica.
3. **Chaves privadas** — de identidade, de sessão, e de transporte, em
   cada dispositivo.
4. **Metadados de conversa** — quem fala com quem, com que frequência,
   aproximadamente quando (proteção parcial — ver secção de limitações).

## Adversários considerados

| Adversário | Capacidades assumidas |
|---|---|
| **Operador do servidor/relay** (curioso ou comprometido) | Acesso total à máquina que corre o `relay_server` e ao Redis — lê tudo o que lá estiver guardado, em qualquer momento. |
| **Autoridade com poder legal sobre o operador** | Pode obrigar o operador a entregar tudo o que o servidor tem, ou a desligar o serviço. Não pode obrigar o operador a entregar o que ele não tem (ex.: chaves privadas de utilizadores, que nunca lá estiveram). |
| **Atacante passivo na rede** (ISP, ponto de trânsito, Wi-Fi público) | Vê todo o tráfego de rede entre cliente e servidor, mas não consegue modificá-lo sem ser detetado. |
| **Atacante ativo na rede** (man-in-the-middle) | Além de ver, tenta modificar, injetar, ou bloquear tráfego. |
| **Adversário com computador quântico futuro** | Grava tráfego cifrado hoje, tenta decifrá-lo daqui a vários anos quando tiver capacidade de quebrar criptografia clássica ("harvest now, decrypt later"). |
| **Um dos participantes da conversa** | **Não é um adversário coberto por este modelo.** Se Alice mostra a Bob as mensagens que Bob lhe enviou, ou grava a conversa, isso está fora do alcance de qualquer sistema de E2EE — é uma limitação física, não técnica. |

## O que está protegido, e como

| Ameaça | Proteção | Estado |
|---|---|---|
| Servidor lê o conteúdo das mensagens | Double Ratchet (`core/src/ratchet.rs`) — o servidor só vê bytes cifrados, nunca teve a chave | ✅ Implementado e testado |
| Servidor descobre quem enviou uma mensagem | Sealed sender (`core/src/sealed_sender.rs`) — o campo `from` é um envelope cifrado que só o destinatário consegue abrir | ✅ Implementado e testado |
| Uma chave de sessão comprometida expõe conversas passadas | Forward secrecy (Double Ratchet) — cada mensagem usa uma chave derivada nova, descartada logo a seguir | ✅ Implementado e testado |
| Uma chave de sessão comprometida expõe conversas futuras indefinidamente | Post-compromise security (Double Ratchet) — a sessão recupera segurança após a troca seguinte de mensagens | ✅ Implementado e testado |
| Atacante na rede lê/modifica o tráfego cliente↔servidor | Noise Protocol (`transport/`) — canal autenticado e cifrado, independente do E2EE de aplicação (defesa em profundidade) | ✅ Implementado e testado |
| Servidor forjar uma pre-key de outro utilizador | Assinatura Ed25519 da signed pre-key, verificada antes de qualquer handshake (`core/src/x3dh.rs`) | ✅ Implementado e testado |
| Alguém com acesso físico ao dispositivo lê a base de dados local | SQLCipher (`storage/`) — base de dados cifrada em repouso | ✅ Implementado e testado |
| Computador quântico futuro decifra tráfego capturado hoje | PQXDH (X25519 + ML-KEM/Kyber combinados) — feature `pq` do `core/` | ✅ Implementado e testado (opcional, exige Rust 1.81+) |

## O que NÃO está protegido (limitações honestas)

Isto é a parte que a maioria dos projetos de "mensageiro seguro" evita
escrever. Aqui está, sem rodeios:

- **O servidor sabe o destinatário (`to`) de cada mensagem.** Precisa
  disso para saber para que fila entregar. Isto significa que um operador
  malicioso ou coagido consegue construir um grafo de "quem recebe
  mensagens de forma consistente", mesmo sem saber quem as enviou nem o
  que dizem.
- **Timing e frequência de tráfego são visíveis ao servidor e a quem
  observar a rede.** Nada no design atual esconde *quando* alguém envia
  uma mensagem, nem com que frequência. Um adversário capaz de observar
  o servidor ao longo do tempo consegue inferir padrões de atividade.
- **Um único servidor é um único ponto de pressão legal.** Se o operador
  for coagido a desligar o serviço, a disponibilidade cai para todos os
  utilizadores desse servidor. A confidencialidade não é afetada (o
  servidor nunca teve as chaves), mas a capacidade de comunicar, sim.
  Federação (vários servidores independentes) resolveria isto — ainda
  não implementada, ver `docs/protocol-spec.md` secção 4.
- **O transporte ainda não usa TLS (`wss://`).** O Noise Protocol cifra e
  autentica a ligação, mas sem TLS por cima não há a camada extra de
  defesa que a maioria dos serviços na internet já assume como padrão.
  Adequado para redes de confiança/testes, não para produção.
- **Comprometimento do dispositivo quebra tudo.** Se o telemóvel de
  alguém tem malware, ou se um atacante tem acesso físico E consegue a
  passphrase da base de dados, todas as conversas desse dispositivo
  ficam expostas. Isto é verdade para qualquer sistema de E2EE — não é
  uma fraqueza específica do QT, mas vale a pena dizê-lo explicitamente.
  A passphrase da base de dados vir do Keystore/Secure Enclave do
  sistema operativo (em vez de um argumento de linha de comandos, como é
  hoje) reduziria este risco — ver `docs/protocol-spec.md` secção 5.
- **Screenshots e capturas de ecrã não podem ser impedidos por nenhuma
  tecnologia de mensagens.** Quem recebe uma mensagem sempre pode
  fotografá-la com outro dispositivo.
- **Sem auditoria de segurança externa.** Todo o código foi escrito e
  revisto só pelo autor original. Isto é uma limitação séria para
  qualquer alegação de segurança "a sério" — ver a secção seguinte.

## Testes adversariais já feitos

Antes de qualquer avaliação externa, faz sentido testar contra o
adversário mais simples de todos: bytes aleatórios ou malformados, do
tipo que um atacante (ou simplesmente um bug noutro ponto da cadeia)
poderia enviar.

**Um problema real foi encontrado e corrigido através deste exercício**:
`cli/src/bin/messenger.rs` fazia `bytes[0]` e outras indexações diretas
sobre mensagens vindas do servidor, sem verificar o tamanho primeiro — uma
mensagem vazia ou demasiado curta entrava em pânico e derrubava o
processo inteiro (uma negação de serviço trivial). Corrigido para
devolver um erro controlado e continuar a processar as restantes
mensagens da fila, em vez de crashar.

Cobertura atual:
- `core/tests/fuzz_lite.rs` — 40.000 tentativas com bytes aleatórios
  contra `RatchetState::from_bytes` e `unseal_sender_identity` (as duas
  funções do núcleo que recebem bytes mais diretamente expostos a um
  adversário de rede), mais um caso de fronteira específico. Zero pânicos.
- `cli/tests/fuzz_lite.rs` — 20.000 tentativas contra
  `deserialize_bundle` (o parsing do bundle de pre-keys recebido de
  outra pessoa através do servidor). Zero pânicos.
- `cli/src/bin/messenger.rs` tem também um teste unitário dedicado que
  tenta todos os tamanhos de 0 a 35 bytes contra a função corrigida.
- `core/fuzz/` tem alvos reais de `cargo-fuzz` (fuzzing guiado por
  cobertura, mais poderoso do que os testes aleatórios acima) prontos
  para correr — **não foram corridos** no ambiente onde este projeto foi
  desenvolvido, por não haver Rust *nightly* disponível nesse ambiente
  específico. Correm normalmente em qualquer máquina com
  `rustup toolchain install nightly` — ver `core/fuzz/README.md` (a criar)
  ou a documentação do [`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz).

**O que isto prova, e o que não prova**: estes testes dão confiança de
que os pontos mais óbvios de entrada de dados não confiáveis não
crasham trivialmente. Não substituem fuzzing guiado por cobertura a
sério (mais tempo, mais iterações, exploração mais inteligente do espaço
de input) nem, principalmente, uma auditoria de segurança feita por
alguém que não seja o autor do código.

- **Anonimato de rede completo** (esconder o endereço IP de quem se
  liga). Precisaria de Tor ou uma mixnet por cima do transporte atual.
  Não está no roadmap imediato.
- **Deteção e prevenção de spam/abuso.** Um sistema com registo anónimo
  é, por natureza, mais vulnerável a abuso do que um com verificação de
  identidade. Este é um trade-off consciente do projeto (ver
  `apresentacao-projeto.md`), não um esquecimento.
- **Proteção contra coerção física do utilizador** ("aponta uma arma e
  pede a passphrase"). Nenhuma tecnologia resolve isto.
- **Grupos e conversas com mais de duas pessoas.** O protocolo atual
  (X3DH + Double Ratchet clássico) é desenhado para conversas 1-para-1;
  grupos exigiriam uma extensão (ex.: Sender Keys, ou MLS) ainda não
  implementada.

## Modelo de confiança, resumido por componente

| Componente | Nível de confiança necessário |
|---|---|
| Dispositivo do utilizador | Total — é onde as chaves privadas vivem. Se este for comprometido, nada mais importa. |
| Rede (Wi-Fi, ISP, internet) | Zero — assume-se hostil por padrão; toda a proteção vem da criptografia, não da rede. |
| Servidor/relay | Zero para conteúdo e remetente; parcial para metadados (`to`, timing) — ver limitações acima. |
| Bibliotecas de terceiros (`x25519-dalek`, `chacha20poly1305`, etc.) | Alta — não são reimplementadas, mas também não foram auditadas especificamente para este projeto (herdam a reputação/maturidade geral do ecossistema Rust, que é considerável, mas não é o mesmo que uma auditoria dedicada). |

## Porque é que isto importa mais do que parece

Um leitor técnico que avalie este projeto não vai (nem deve) confiar
apenas na palavra de que "é seguro". Este documento existe para que essa
avaliação possa ser feita com base no que está escrito aqui: o que foi
pensado, o que foi decidido deixar de fora, e porquê. Se alguma coisa
aqui parecer errada ou incompleta, é exatamente esse o tipo de feedback
que este projeto precisa — ver `SECURITY.md` para como reportar.
