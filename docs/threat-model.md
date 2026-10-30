# Modelo de Ameaças

> Descreve o que o código faz hoje (versão de testes, apps Android e iPhone 0.9.51), o que
> os testes demonstram e, com igual peso, o que **não** está protegido.

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
| **Outro relay da federação** (curioso ou comprometido) | Recebe os pedidos do relay par: onde está uma conta, mensagens e anexos para as contas dele, bundles e partes da cópia de recuperação. Pode mentir sobre onde está uma conta, atrasar ou descartar o que lhe pedem. |
| **Atacante passivo na rede** | Vê todo o tráfego cliente↔relay e relay↔relay. |
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
| Relay descobre o remetente | Sealed Sender: identidade do remetente numa caixa selada para o destinatário; o cabeçalho PQXDH **não** contém a identidade; o relay não guarda o remetente (a ligação de quem envia é autenticada, por isso um relay mal-intencionado pode associá-la no momento) | `ffi/tests/client_flow.rs::sealed_sender_relay_nao_ve_plaintext_nem_remetente`; mesmo teste no Redis real |
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
| Relay das chamadas liga o vídeo de uma sala a uma conta (teto do vídeo, 0006) | Fichas cegas (VOPRF, RFC 9497, ristretto255-SHA512, crate `voprf`): o emissor só vê elementos aleatórios; o resgate só leva nonce e saída; o relay das chamadas não acede às contas e só guarda um resumo do nonce | `server/tests/fichas_e2e.rs::ficha_aceite_uma_vez_e_o_relay_nao_liga_resgate_a_emissao`, `core::fichas::tests::o_emissor_so_ve_elementos_aleatorios` |
| Mais vídeo pelo relay do que o teto da conta | Contagem por conta e por mês na emissão (atómica: nunca passa do teto, nem com pedidos ao mesmo tempo); gasto duplo no Redis; fichas forjadas, repetidas ou de outro mês recusadas | `emissao_nunca_passa_do_teto_por_conta_e_por_mes`, `pedidos_ao_mesmo_tempo_nao_passam_do_teto`, `ficha_forjada_de_outro_mes_ou_de_outra_chave_recusada`, `sem_ficha_so_voz_com_ficha_um_bloco_de_video` |
| Emissor marca uma conta com uma chave só dela (partição de chaves) | Chave de cada mês publicada fora do relay (`https://sealerapp.com/estado.json`, Cloudflare). A app verifica a prova DLEQ, recusa a emissão se a chave não for a do site e só gasta fichas se a chave da sala também for essa; sem a chave do site falha fechada (vídeo pelo relay em pausa, voz e direto normais). O mês dito pelo relay só se segue se for o vizinho, a menos de 48 h da viragem (partição pelo mês, com os 24 meses publicados) | `prova_dleq_obrigatoria`, `fichas_de_outro_mes_ou_de_outra_chave_nao_servem`, `ffi::fichas::tests::chave_diferente_da_publicada_na_emissao_e_recusada`, `ffi::fichas::tests::sem_chave_do_site_nada_se_pede`, `call::fichas::tests::chave_da_sala_diferente_da_publicada_nada_se_gasta`, `call::fichas::tests::sem_chave_do_site_falha_fechada`, `call/tests/fichas_e2e.rs::chave_diferente_da_publicada_ou_sem_site_falha_fechada`, `mes_do_relay_so_o_proprio_ou_o_vizinho_na_viragem`, `ffi::fichas::tests::so_meses_plausiveis_vao_para_o_motor` |

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
  * ML-KEM usa a implementação `ml-kem 0.2.3` (RustCrypto); as garantias de
    tempo constante não foram verificadas por nós.
  * Desde a app 0.9.3 (PQXDH v2) o HKDF inclui a transcrição (todas as chaves
    públicas e o ciphertext KEM). Sessões iniciadas antes continuam com o v1.

## 4. O que NÃO está protegido (limitações)

* **Dispositivo comprometido** (root, malware com acesso à memória, depuração
  da app): a chave do storage, as chaves de sessão e o plaintext estão em
  memória; nenhuma defesa de software no Rust resolve isto. A chave do storage
  passa pela memória da JVM no arranque.
* **Endpoint do outro participante** comprometido, screenshots, capturas.
* **Metadados no relay:** o relay vê o destinatário (`to`), o tamanho, o
  instante e o endereço IP de cada ligação (o relay **não** os regista, mas um
  operador mal-intencionado pode). Sealed Sender tira o remetente do envelope guardado, mas
  a ligação de quem envia é autenticada: um operador mal-intencionado pode associar
  a conta que envia a cada envelope no momento. Também não protege contra análise de
  tráfego/correlação temporal.
* **Ligação direta nas chamadas** (automática desde o 0006, 23/10/2026): entre contactos mútuos e
  não bloqueados, já na primeira chamada e sem perguntar nada, **cada lado vê o IP do outro**
  (público e locais, com a porta), depois de atender. A app não tem janela de aviso nem opção para
  o evitar (decisão do Nuno). Não há verificação de contactos: confia-se que o ID guardado é de
  quem se pensa. Se um dos lados não tiver o outro nos contactos, ou o bloqueou, tudo passa pelo
  relay; bloquear, apagar ou tirar dos contactos a meio volta logo ao relay.
* **Fichas do vídeo pelo relay (0006):**
  * O emissor (relay das mensagens) vê quando cada conta pede fichas e quantas; o relay das
    chamadas vê quando cada sala gasta (uma por minuto de vídeo pelo relay) e o IP. Os dois
    processos estão na mesma máquina e são do mesmo operador: um operador mal-intencionado pode
    tentar ligar o pedido das fichas a uma chamada pelo IP e pela hora. Por isso a app pede numa
    hora aleatória, fora das chamadas; isto não impede a correlação pelo IP (o mesmo telemóvel na
    mesma rede).
  * A chave de cada mês vem do site (Cloudflare), não do relay: um relay comprometido sozinho não
    consegue dar uma chave diferente a cada conta. Fica a confiança no site: quem controlar o
    repositório do site ou a conta da Cloudflare **e** o relay podia servir a cada telemóvel uma
    lista diferente (pelo IP). Não há registo público de chaves (transparência) nem comparação
    entre utilizadores. O ficheiro é publicado à mão pelo operador: se o deploy criar uma semente
    nova e o site não for atualizado, o vídeo pelo relay fica em pausa para todos (falha fechada).
  * Cada conta registada recebe o teto do mês: quem criar muitas contas (cada uma com prova de
    trabalho) junta mais fichas.
  * A semente das chaves está em texto simples no disco do relay (0600, como a chave Noise): quem
    a tiver emite fichas à vontade, para todos os meses (as chaves derivam dela).
  * Sem fichas, os pacotes de vídeo de tamanho mínimo (relatórios de quem só vê) passam na mesma,
    até 5 por segundo e por lado (~1,5 kB/s): uma app alterada podia usá-los para um vídeo
    minúsculo.
  * Redis em baixo ou sem semente: nenhuma ficha é aceite (vídeo pelo relay em pausa; voz e direto
    normais).
* **O relay pode censurar, atrasar, descartar ou reordenar** (disponibilidade
  não é garantida). Na federação, o relay de casa de cada conta pode fazer o mesmo ao que lhe chega do
  outro relay.
* **Federação (desde 29/10/2026):** federação fechada (`/etc/qt/federacao.json`): ligação `wss://<par>/fed`
  com Noise_XX e a chave fixada dos dois lados, e o HAProxy só deixa passar os IPs da lista. O que muda:
  * O relay par fica a saber que alguém do outro relay procura a conta X (não quem), quando chega uma
    mensagem do outro relay para uma conta sua (o remetente continua selado) e que anexos ou cópias de
    recuperação lhe pedem. A cache "a conta está no relay X" fica só em memória (24 h).
  * O relay de casa põe na fila o que o par entrega sem voltar a verificar quem envia: confia que o par
    já aplicou os limites. Um relay da lista comprometido pode encher as filas das contas do outro.
  * Um relay da lista pode dizer que uma conta é sua e desviar os pedidos de bundle dessa conta. As
    apps continuam a verificar o bundle contra o ID (ver secção 3): não consegue fingir ser a conta, só
    impedir que lhe cheguem mensagens.
  * Os dois relays são do mesmo operador e usam a mesma semente das fichas do vídeo.
* **Spam / abuso:** qualquer conta registada pode enfileirar mensagens para
  qualquer `user_id` registado (limite de 1000 mensagens por fila; desde a 0.9.17
  o relay recusa escritas de contas sem registo). Desde a 0.9.3 há limites por
  conta (300 envelopes por minuto) e uma prova de trabalho ao criar conta (alguns
  segundos de CPU), mas um atacante com muitas máquinas continua a conseguir criar
  contas. Não há consentimento de contacto no relay. Desde a 0.9.18 há denúncias
  (cifradas para o operador) e banimentos por ID; um ID banido pode ser trocado
  por outro novo.
* **Denúncias:** o operador lê as mensagens que quem denuncia escolheu enviar. Não
  há prova de autoria dessas mensagens (uma app alterada pode inventá-las), por
  isso uma denúncia é um indício, não uma prova.
* **Chave de recuperação (desde a 0.9.19):** as 12 palavras dão a conta inteira a
  quem as tiver (ID, contactos, bloqueados). O relay guarda a cópia cifrada
  (XChaCha20-Poly1305, chave por Argon2id) e vê que conta a enviou e quando.
* **One-time prekeys e validade dos bundles (desde a 0.9.3):** o relay entrega uma
  one-time prekey por pedido e o bundle tem data assinada (recusado com mais de 90
  dias). Quem pede bundles em massa pode esgotar as one-time prekeys de alguém (o
  limite por conta atrasa isto); sem elas o PQXDH continua válido.
* **Rollback do armazenamento:** quem restaurar uma cópia antiga do ficheiro
  SQLCipher faz o ratchet recuar (reabrindo janelas de replay/chaves antigas).
  O SQLCipher protege confidencialidade e integridade das páginas, não a
  *frescura*.
* **Ratchet:** chaves de mensagens puladas ficam no estado até serem usadas
  (o roubo do estado nesse momento expõe-nas); no máximo 1000 por cadeia e
  2000 no total. (As mensagens atrasadas da cadeia DH anterior já não se
  perdem: o cabeçalho leva `PN`.)
* **Tamanho:** plaintext máximo de 10 000 bytes por mensagem de texto (limite
  do enquadramento Noise + JSON). Imagens e GIFs até 25 MiB vão cifrados por
  ficheiro, com o tamanho arredondado; o relay vê o tamanho aproximado e quem
  envia e descarrega cada ficheiro (ver `known-limitations.md`, 36).
* **Zeroização parcial:** o segredo inicial e a chave de storage são
  zeroizados nos caminhos controlados pelo QT; as chaves privadas das
  dependências (`x25519-dalek`, `ml-kem`) e cópias feitas pela JVM/alocador
  não são garantidamente limpas.
* **Reinício de sessão incompleto:** uma sessão nova e autêntica é adotada
  automaticamente (sessões cruzadas, contacto que apagou e voltou a
  escrever), mas falta o caso em que quem apagou o contacto recebe primeiro
  (ver `known-limitations.md`, 10). Uma reinstalação da app gera uma
  identidade nova.
* **Comunidade (0.9.52):** a lista (índice e fotos) é cifrada com uma chave que vai dentro das
  apps (`COMMUNITY_KEY`). Protege a lista no armazenamento da AWS e de quem não tem o Sealer, mas
  **não de quem instala a app**: qualquer pessoa com a app (ou com a chave tirada dela) pode ver
  as fotos e as medalhas de quem tem medalha. Isto está aceite (decisão do Nuno): a lista mostra
  só foto e medalha, sem nome nem ID, e é o mesmo que qualquer utilizador vê no ecrã. A
  autenticidade não depende desta chave: o índice é assinado com a chave das medalhas, que nunca
  sai do PC do operador. O relay vê o número da lista, quando cada conta a pede e quantas fotos
  pede, e que contas enviaram uma foto (só pelo contador diário, sem ligar a conta ao envio).
  Quem tem uma foto de perfil igual nos contactos e na Comunidade pode ser reconhecido por um
  contacto que já a conhece; por isso entram só pessoas com medalha, que o Nuno avisa fora da app.
* **Verificação de identidade fora de banda:** o QT Public ID tem de ser
  trocado por um canal autêntico (pessoalmente, o código QR resolve isto). As
  apps mostram um *safety number* de 60 dígitos para comparar por outro
  canal, mas ninguém é obrigado a fazê-lo.
* **`ws://` só em testes locais** (builds de debug). As builds de release só
  aceitam `wss://`; o Noise protege o conteúdo e o TLS evita expor o padrão
  Noise.
* **Sem análise de canais laterais nem fuzzing contínuo.** Há fuzzing com
  `cargo-fuzz` desde a 0.9.17 (5 alvos, corrido à mão); os testes de entradas
  inválidas e por propriedades estão na secção 3.
* **Legacy:** a feature `legacy-api` da FFI expõe chaves privadas e estados
  em bytes. Nunca deve ser ativada na build Android (o script de bindings
  falha se aparecer no Kotlin).

## 5. Modelo de confiança por componente

| Componente | Confiado para… | Não confiado para… |
|---|---|---|
| Rust `core` (+ dependências) | Toda a criptografia | - |
| Android Keystore / Secure Enclave do iPhone | Proteger a chave que cifra o storage | Resistir a um dispositivo comprometido |
| Operador (chave das denúncias e dos banimentos) | Ler as denúncias, banir e emitir medalhas | Ler outras mensagens (não tem como) |
| Relay + Redis | Entregar o que recebe, quando quiser | Confidencialidade, autenticidade de bundles, ordem, disponibilidade |
| Relay par da federação | Entregar às suas contas o que o outro relay lhe passa; dizer onde está cada conta | Confidencialidade, quem envia, autenticidade de bundles |
| Emissor das fichas (relay das mensagens) | Contar e emitir fichas do vídeo | Saber que fichas deu (vê-as cegas); escolher a chave do mês (vem do site) |
| Site (`sealerapp.com/estado.json`, Cloudflare) | Publicar a mesma chave das fichas a toda a gente | - |
| Relay das chamadas + `qt-fichas` no Redis | Recusar fichas gastas ou forjadas | Saber de que conta é uma ficha (não tem como) |
| Rede | Nada | Tudo |
| Apps Kotlin e Swift | Mover bytes, UI, ciclo de vida, preparar imagens antes de as cifrar | Qualquer criptografia (proibido) |

## 6. Revisão de segurança interna

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
  prolongado, revisão manual de dependências (o CI corre o `cargo audit`).
