# Modelo de Ameaças (v0.2: fase Android)

> **Estado: protótipo técnico, NÃO auditado.** Nada neste documento é uma
> alegação de segurança de produção. Descreve o que o código faz hoje, o que
> os testes demonstram e, com igual peso, o que **não** está protegido.
> Uma auditoria criptográfica externa continua a ser necessária antes de o QT
> ser apresentado como sistema de comunicação seguro.

## 1. Ativos

1. **Conteúdo das mensagens.**
2. **Identidade do remetente** (quem enviou uma mensagem concreta).
3. **Chaves privadas**: identidade X25519/Ed25519, signed pre-key, prekey
   ML-KEM, one-time prekeys, estado do ratchet, chave estática Noise.
4. **Metadados**: quem fala com quem, quando, quanto (proteção só parcial).
5. **Estado de sessão** (ratchet, deduplicação de envelopes).

## 2. Adversários

| Adversário | Capacidades assumidas |
|---|---|
| **Relay comprometido / curioso** | Acesso total ao processo, ao Redis e ao disco do relay; pode registar, atrasar, duplicar, reordenar, descartar ou fabricar envelopes e bundles. |
| **Atacante passivo na rede** | Vê todo o tráfego cliente↔relay. |
| **Atacante ativo na rede** | Injeta, repete, reordena, corta ou modifica frames. |
| **Quem lê o Redis / o armazenamento do relay** | Cópia total do que o relay persiste. |
| **Quem captura tráfego hoje (harvest-now-decrypt-later)** | Guarda tudo; mais tarde tem um computador quântico capaz de correr Shor. |
| **Quem obtém uma cópia do ficheiro SQLCipher** | Ficheiro cifrado, sem a chave. |
| **Quem obtém o *estado* de uma sessão num instante T** | Cópia do estado do ratchet nesse momento. |
| *Fora de âmbito* | Dispositivo totalmente comprometido; o outro participante da conversa; ver §4. |

## 3. O que o QT protege, e a evidência

Cada linha indica o mecanismo e **o teste que o demonstra** (corre com
`cargo test --workspace`; os testes de relay precisam de Redis).

| Ameaça | Mecanismo | Evidência |
|---|---|---|
| Relay lê o conteúdo | Double Ratchet; o relay só guarda envelopes opacos | `server/tests/two_clients_over_relay.rs` lê o Redis real e verifica que não há plaintext |
| Relay descobre o remetente | Sealed Sender: identidade do remetente numa caixa selada para o destinatário; o cabeçalho PQXDH **não** contém a identidade | `ffi/tests/client_flow.rs::sealed_sender_relay_nao_ve_plaintext_nem_remetente`; mesmo teste no Redis real |
| Relay substitui o bundle de outra pessoa | `start_session` verifica o bundle contra o QT Public ID (chaves de identidade) e a assinatura Ed25519, que cobre a signed pre-key **e** a chave ML-KEM (impede um relay de anular a proteção pós-quântica trocando-a); chaves ML-KEM validadas (tamanho e codificação canónica) | `bundles_invalidos_sao_rejeitados`, `relay_malicioso_nao_pode_substituir_a_chave_mlkem` |
| Atacante passivo lê o tráfego | Noise XX (X25519, ChaChaPoly, SHA256) + E2EE por cima | `transport/` e `relay_hardening.rs` |
| Atacante ativo corrompe, repete ou reordena frames de transporte | O Noise tem contador de nonce estrito: qualquer frame corrompido/repetido/fora de ordem faz o relay encerrar a sessão | `frame_noise_corrompido…`, `frame_noise_repetido…`, `frame_noise_fora_de_ordem…` |
| Reutilização de uma assinatura de autenticação noutra sessão | A autenticação do relay assina `contexto ‖ hash do handshake Noise` (channel binding) | `assinatura_de_autenticacao_nao_serve_noutra_sessao` |
| Utilizador A lê/apaga a fila de B ou regista o bundle de B | `user_id` autenticado por assinatura Ed25519; o relay só aceita operações sobre o próprio `user_id` | `utilizadores_nao_acedem_a_filas_ou_bundles_de_outros` |
| Perda de mensagens por queda de rede | Entrega *at-least-once*: o relay só apaga após `AckMessages` | `entrega_at_least_once_so_apaga_apos_ack`, `conversa_a_b_pelo_relay_com_quedas_e_reinicios` |
| Replay / duplicados de envelopes | Deduplicação persistente por (chave DH do ratchet, n) antes de decifrar + chaves de mensagem de uso único | `replay_e_rejeitado_tambem_depois_de_reiniciar`, `message_key_usada_nao_pode_ser_reutilizada` |
| Manipulação de ciphertext | AEAD (ChaCha20-Poly1305) com o cabeçalho (chave DH, n) como AAD; falha não altera o estado | `ciphertext_alterado_falha_e_nao_estraga_o_estado`, `decrypt_falhado_nao_altera_o_estado` |
| Mensagem forjada para quebrar a sessão (DoS) | `RatchetState::decrypt` é transacional; a FFI trabalha sobre cópias do disco | `decrypt_falhado_nao_altera_o_estado`, `cabecalho_forjado_nao_gasta_a_prekey` |
| One-time prekey reutilizada / consumida por um forjador | Só é apagada depois de a 1.ª mensagem autenticar | `prekey_reutilizada_e_rejeitada`, `cabecalho_forjado_nao_gasta_a_prekey` |
| Estado comprometido expõe mensagens passadas | Forward secrecy do Double Ratchet (chaves de mensagem apagadas após uso) | `core/tests/forward_secrecy.rs` |
| Estado comprometido expõe mensagens futuras | Post-compromise security após um novo passo DH (limites em §4) | `post_compromise_security_cura_apos_novo_passo_dh` |
| Cópia do ficheiro da base de dados | SQLCipher com chave binária de 32 bytes vinda do Android Keystore; sem cabeçalho SQLite em claro. O histórico de mensagens é guardado **só** dentro da base cifrada (com `secure_delete`), nunca em claro | `storage/tests/storage_extended.rs::texto_das_mensagens_nunca_fica_em_claro_no_ficheiro` |
| Lixo / entradas malformadas | Todas as entradas de rede e disco devolvem erro (sem `unwrap` em caminhos de rede/cripto); limites de tamanho | `sessao_inexistente_e_lixo_nao_fazem_panic`, `limites_e_validacao_de_entrada`, fuzz do estado em `forward_secrecy.rs` |

### 3.1 Segurança pós-quântica: o que exatamente se afirma

O segredo inicial da sessão é
`HKDF-SHA256(DH1 ‖ DH2 ‖ DH3 [‖ DH4] ‖ ss_ML-KEM-768)`, com info `PQXDH_v1`,
e depois alimenta a root key do Double Ratchet.

* **Afirmado:** *confidencialidade do segredo inicial contra harvest-now-
  decrypt-later*. Um adversário que quebre **só** X25519 (computador
  quântico) mas **não** ML-KEM-768 não obtém o segredo inicial; um que quebre
  só ML-KEM mas não X25519 também não. Como as root/chain keys seguintes são
  derivadas desse segredo (as saídas DH posteriores são *misturadas* na root
  key, não a substituem), o conteúdo das mensagens da sessão depende do
  segredo ML-KEM durante toda a conversa. Testado: o segredo diverge se o
  componente clássico **ou** o pós-quântico for alterado
  (`segredo_depende_do_componente_*`).
* **NÃO afirmado:**
  * **Autenticação pós-quântica.** Assinaturas (Ed25519) e DH de identidade
    são clássicos: um adversário quântico *ativo* no futuro poderia falsificar
    identidades (não decifra o tráfego antigo).
  * **Cura pós-quântica.** O *post-compromise security* assenta em X25519; um
    adversário quântico que obtenha o estado e depois quebre X25519 pode
    acompanhar a sessão.
  * **Sealed Sender** usa X25519: a identidade do remetente protege-se contra
    adversários clássicos, **não** contra harvest-now-decrypt-later quântico.
  * **O transporte Noise é clássico.** Só o E2EE (conteúdo) é híbrido.
  * ML-KEM é uma implementação (`ml-kem 0.2.3`) **sem auditoria independente**
    e sem garantias de tempo constante verificadas por nós.
  * Sem *transcript binding*: o HKDF não inclui as chaves públicas nem o
    ciphertext KEM (mantido para compatibilidade com o formato v0.1).

## 4. O que NÃO está protegido (limitações)

* **Dispositivo comprometido** (root, malware com acesso à memória, depuração
  da app): a chave do storage, as chaves de sessão e o plaintext estão em
  memória; nenhuma defesa de software no Rust resolve isto. A chave do storage
  passa pela memória da JVM no arranque.
* **Endpoint do outro participante** comprometido, screenshots, capturas.
* **Metadados no relay:** o relay vê o destinatário (`to`), o tamanho, o
  instante e o endereço IP de cada ligação (o relay **não** os regista, mas um
  operador mal-intencionado pode). Sealed Sender esconde só o remetente, e só
  contra o relay, não contra análise de tráfego/correlação temporal.
* **O relay pode censurar, atrasar, descartar ou reordenar** (disponibilidade
  não é garantida). Não há federação.
* **Spam / abuso:** qualquer utilizador autenticado pode enfileirar mensagens
  para qualquer `user_id` (limite de 1000 mensagens por fila). Não há *rate
  limiting*, nem consentimento de contacto no relay.
* **Sem one-time prekeys servidas pelo relay:** o relay entrega o mesmo
  bundle a todos (sem OTPK). O PQXDH sem OTPK é válido mas perde a proteção
  extra contra replay do handshake inicial.
* **Bundles sem validade:** um relay pode servir um bundle antigo (signed
  pre-key e prekey PQ não rodam automaticamente nem têm timestamp).
* **Rollback do armazenamento:** quem restaurar uma cópia antiga do ficheiro
  SQLCipher faz o ratchet recuar (reabrindo janelas de replay/chaves antigas).
  O SQLCipher protege confidencialidade e integridade das páginas, não a
  *frescura*.
* **Ratchet:** mensagens da cadeia DH anterior que cheguem *depois* de uma
  mensagem da cadeia nova perdem-se (o cabeçalho não tem `pn`); chaves de
  mensagens puladas ficam no estado até serem usadas (o roubo do estado nesse
  momento expõe-nas); `MAX_SKIP` = 1000.
* **Tamanho:** plaintext máximo de 10 000 bytes por mensagem de texto (limite
  do enquadramento Noise + JSON). Imagens e GIFs até 25 MiB vão cifrados por
  ficheiro, com o tamanho arredondado; o relay vê o tamanho aproximado e quem
  envia e descarrega cada ficheiro (ver `known-limitations.md`, 36).
* **Zeroização parcial:** o segredo inicial e a chave de storage são
  zeroizados nos caminhos controlados pelo QT; as chaves privadas das
  dependências (`x25519-dalek`, `ml-kem`) e cópias feitas pela JVM/alocador
  não são garantidamente limpas.
* **Reinstalação do contacto:** se o outro lado perder o estado e iniciar uma
  sessão nova, o cabeçalho PQXDH novo é ignorado enquanto existir sessão
  antiga (não há fluxo de *session reset* nem *safety numbers*).
* **Verificação de identidade fora de banda:** o QT Public ID tem de ser
  trocado por um canal autêntico. Não há comparação de impressões digitais na
  interface.
* **Transporte de teste sem TLS** (`ws://`). Em produção usar `wss://` (o
  Noise protege o conteúdo, mas TLS evita expor o padrão Noise e aproveita a
  infraestrutura de certificados).
* **Sem auditoria externa**, sem análise de canais laterais, sem fuzzing
  contínuo; o código foi escrito e revisto por um único autor (e assistido por
  IA).
* **Legacy:** a feature `legacy-api` da FFI expõe chaves privadas e estados
  em bytes. Nunca deve ser ativada na build Android (o script de bindings
  falha se aparecer no Kotlin).

## 5. Modelo de confiança por componente

| Componente | Confiado para… | Não confiado para… |
|---|---|---|
| Rust `core` (+ dependências) | Toda a criptografia | - (sem auditoria) |
| Android Keystore | Proteger a chave que cifra o storage | Resistir a um dispositivo comprometido |
| Relay + Redis | Entregar o que recebe, quando quiser | Confidencialidade, autenticidade de bundles, ordem, disponibilidade |
| Rede | Nada | Tudo |
| App Kotlin | Mover bytes, UI, ciclo de vida | Qualquer criptografia (proibido) |

## 6. Auditoria interna realizada nesta fase

* Procuradas: passwords/chaves/segredos fixos (nenhum), RNG (só `OsRng`),
  `unwrap`/`expect`/`panic` em rede/cripto (removidos de `core`, `storage`,
  `server`, `ffi`, `transport`; restam `expect` apenas nos binários de demo/CLI),
  logs de plaintext/chaves (nenhum; o relay deixou de registar endereços IP e
  oculta credenciais do URL do Redis), reutilização de nonce (nonces AEAD
  aleatórios de 96 bits sob chaves de mensagem de uso único; ephemeral novo
  por envelope selado), downgrade (a API de alto nível só aceita PQXDH; o
  bundle de formato fixo não permite remover a chave ML-KEM), serialização
  (parsers com verificação de tamanhos; testes de truncagem/bit-flip).
* Corrigido: bundle PQXDH com a chave ML-KEM fora da assinatura (relay podia anular a proteção pós-quântica), relay sem autenticação, entrega destrutiva, `decrypt` não
  transacional, SQLCipher dependente do sistema (inutilizável no Android),
  API legacy a expor chaves, permissões da chave Noise do relay.
* **Não** feito: revisão formal, análise de tempo constante, fuzzing
  prolongado, revisão de dependências (`cargo audit` não corrido).
