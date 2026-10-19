# QT

<p align="center"><img src="branding/sealer-logo-transparente.png" width="140" alt="Logotipo SEALER"></p>

As apps chamam-se **SEALER**; QT é o nome do protocolo e deste repositório.

**Versão de testes (Android 0.9.11, iPhone 0.9.3), não auditada.** Um mensageiro E2EE com criptografia
pós-quântica (X3DH, PQXDH, Double Ratchet, Sealed Sender, Noise Protocol),
tudo implementado em Rust, com apps Android e iPhone e relay na nuvem, já
a funcionar de ponta a ponta. **Ainda não é um produto acabado nem deve ser
usado para informação sensível.** Ver a secção "Sê honesto comigo: isto
já funciona?" já a seguir.

[Read this in English →](README.en.md)

<p align="center">
  <img src="site/public/ecras/conversas-0.9.11.jpg" width="180" alt="Ecrã inicial: cabeçalho roxo com o logo, a foto de perfil e o ID, e a lista de conversas">
  <img src="site/public/ecras/chat-0.9.11.jpg" width="180" alt="Uma conversa cifrada ponta a ponta">
  <img src="site/public/ecras/definicoes-0.9.11.jpg" width="180" alt="Ecrã Definições">
  <img src="site/public/ecras/novo-contacto-0.9.11.jpg" width="180" alt="Ecrã Novo contacto com o código QR">
</p>
<p align="center"><sub>Novo design da app Android (0.9.11). Os nomes e as mensagens são exemplos.</sub></p>

## Contexto

Este projeto nasceu de um interesse genuíno em privacidade e segurança de
sistemas, em particular na questão de como construir comunicação em que a
privacidade é garantida pelo desenho técnico e não só por uma política
escrita (a "proteção de dados desde a conceção" que o RGPD pede). Em vez de
ficar só na teoria, o objetivo foi
implementar e testar cada peça a sério: não pseudocódigo, não diagramas, mas
código que compila, corre, e tem testes automáticos a confirmar que faz o
que diz que faz.

## Porque é que isto existe (o objetivo, para além do exercício técnico)

Quase todos os mensageiros populares pedem o teu número de telefone para te registares. Mesmo quando o conteúdo das mensagens é privado, esse registo cria um rasto: quem fala com quem, quando, com que frequência. Para empresas, isto é também uma questão de conformidade: o RGPD exige proteção de dados desde a conceção e minimização de dados, e cada dado que um serviço guarda é um dado que pode ser exposto numa fuga.

O QT tenta responder a isto com engenharia, não só com uma política de privacidade: identidade baseada numa chave criptográfica (sem telefone, sem email), cifra ponta a ponta que nem o servidor consegue quebrar, e código aberto para que qualquer pessoa possa verificar que não há truques escondidos.

## Sê honesto comigo: isto já funciona?

Sim, de ponta a ponta, em versão de testes (**app Android 0.9.11 e app iPhone 0.9.3**), com testes fechados entre poucas pessoas. Não foi auditado de forma independente e não deve ser usado para conversas reais ou informação sensível.

### O que a app já faz (19 de outubro de 2026)

- **Chamadas de voz cifradas ponta a ponta** no Android, testadas entre dois telemóveis com muito boa qualidade. Tocam com a app fechada ou o telemóvel bloqueado (desde a 0.9.6), com um toque próprio da SEALER (0.9.8). Ver "Chamadas de voz" mais abaixo.
- **Conversas a dois e em grupo** (até 10 pessoas, 3 administradores), entre Android e iPhone, com histórico cifrado no telemóvel e avisos de *Enviado / Entregue / Visto*.
- **Fotos, GIFs, mensagens de voz e ficheiros até 25 MB** (PDF, documentos...), a dois e nos grupos. Cada ficheiro é cifrado no telemóvel com uma chave própria e apagado do relay quando é descarregado, ou ao fim de 24 horas.
- **Notificações push com a app fechada:** um aviso genérico ("Tens mensagens novas"), sem o nome de quem envia nem o texto. Desde a Android 0.9.6 a app decifra em segundo plano, no telemóvel, para as chamadas tocarem com a app fechada; com o bloqueio da app ativo nada é decifrado sem o código.
- **Convites por código QR**, reações, bloquear contactos, online / offline só para os contactos (cifrado) e bloqueio da app com código próprio, impressão digital ou rosto (por agora só no Android).
- **Foto de perfil** (0.9.9): só os teus contactos a veem, cifrada ponta a ponta dentro da conversa (o servidor nem sabe que é uma foto). A original pode ter até 20 MB (0.9.11), vês a foto antes de a aplicar, e só se pode mudar de 72 em 72 horas.
- **Seis línguas** (0.9.10): português, inglês, francês, espanhol, alemão e italiano. Segue a língua do telemóvel, ou escolhe-se em Definições, "Idioma".
- **Novo design** (0.9.11): cabeçalho roxo com o fundo curvo, o logo a rodar como um pião no ecrã inicial, a tua foto e o teu ID no topo, ecrã de Definições (em vez do menu de três pontos) e ecrã Novo contacto com o teu código QR. Por agora só no Android.
- **Apagar a conta** no telemóvel e no servidor, com um toque.
- **Segurança do protocolo (0.9.3):** PQXDH v2 (X25519 + ML-KEM-768), chaves que rodam a cada 7 dias e chaves de uso único servidas pelo relay, enchimento das mensagens (o relay não distingue mensagens curtas de recibos), prova de trabalho para contas novas e limites por conta no relay. As apps 0.9.3 não falam com versões anteriores.

### Testado

- Núcleo criptográfico, transporte, relay, armazenamento local e FFI em Rust, com a suite de testes a passar (`cargo test --workspace`).
- Relay numa instância AWS EC2 em Estocolmo, em `wss://relay.sealer.pt` (HAProxy com certificado Let's Encrypt), com o Redis só em `127.0.0.1` e protegido por ACL.
- App Android em versão *release* assinada, testada em três Samsung e no emulador; app iPhone 0.9.3 compilada e testada num iPhone.
- Teste de campo com cinco aparelhos em três plataformas (2 Android reais, 1 emulador Android, 1 iPhone e 1 Mac), com envios simultâneos e contínuos: nenhuma mensagem perdida. Ver [`docs/field-tests.md`](docs/field-tests.md).
- Relay com 8 000 ligações em simultâneo através do HAProxy sem falhas (cerca de 40 KB por ligação). Ver "O relay em números" mais abaixo.

### O que ainda falta

- **Chamadas de voz no iPhone**, e no Android a integração com o sistema de chamadas (auricular Bluetooth, carro, chamada normal ao mesmo tempo).
- **Foto de perfil, seis línguas e o novo design no iPhone** (por agora só no Android).
- **Distribuição pela Google Play e pela App Store** (hoje a app é instalada à mão pelos testadores). Já preparado: eliminação de conta, política de privacidade e respostas dos formulários ([`docs/google-play.md`](docs/google-play.md)).
- **Infraestrutura de produção:** redundância, backups e alertas do servidor.
- **Auditoria de segurança independente.**

**Próximas etapas:** *testes fechados* (agora, 0.9.x) → *beta* (Google Play e App Store, mais testadores) → *1.0* (depois de auditoria externa).

## Arquitetura

<!-- Gerado por site/scripts/architecture_diagram.py (ver site/README.md, secção 14). -->
![Arquitetura da SEALER: as apps de Android e iPhone usam o mesmo núcleo QT em Rust e trocam envelopes cifrados através de um relay na AWS (HAProxy, relay em Rust e Redis). O site sealer.pt está à parte, na Cloudflare.](docs/arquitetura.svg)

## O que já está construído

| Peça | O que faz | Confiança |
|---|---|---|
| `core/` | Estabelecimento de sessão (X3DH + variante pós-quântica) e cifra contínua (Double Ratchet), o mesmo desenho usado pelo Signal | ✅ Testado (mais de 40 testes automáticos, incluindo casos que devem falhar propositadamente) |
| `transport/` | Proteção da ligação em rede (Noise Protocol) sobre WebSocket | ✅ Testado com ligações WebSocket reais, não simuladas |
| `server/` | O "carteiro": entrega mensagens sem nunca ver o conteúdo nem saber quem as enviou, e guarda as imagens cifradas no máximo 24 horas. Inclui, à parte, o relay das chamadas de voz (UDP, salas anónimas) | ✅ Testado, incluindo sobreviver a ser desligado à força (`kill -9`) e com a lista de comandos do Redis de produção; o relay das chamadas está em produção |
| `call/` | O motor das chamadas de voz: Opus 1.6 com DRED e PLC neural, jitter buffer adaptativo e ligação ao relay das chamadas (UDP, com reserva por WebSocket) | ✅ Testado em Rust, com chamadas de ponta a ponta por um relay local e perdas simuladas; em uso na app Android desde a 0.9.4 |
| `storage/` | Guarda a tua identidade e conversas no disco, cifradas | ✅ Testado, incluindo confirmar que uma password errada não abre nada |
| `cli/` | O mensageiro no terminal (`messenger`), com o mesmo núcleo das apps, e uma demonstração visível do protocolo (`messenger_demo`) | ✅ Testado de ponta a ponta com um relay real: terminal ↔ terminal e terminal ↔ cliente igual ao das apps |
| `ffi/` | A ponte que liga tudo isto a Kotlin (Android), Swift (iOS) e Python: histórico, fila de saída, imagens, bloqueio da app | ✅ Testado em Rust (incluindo sessões cruzadas e imagens) e a partir do Python |
| `android/` | App Android (Kotlin/Compose); toda a criptografia continua no Rust | ✅ Versão 0.9.11 (chamadas de voz, foto de perfil, seis línguas, novo design), release assinada testada em telemóveis reais através do relay na AWS |
| `ios/` | App iPhone (SwiftUI), igual à Android, com o mesmo núcleo Rust | ✅ Versão 0.9.3 compilada num Mac e testada num iPhone, a trocar mensagens com as apps Android através do relay na AWS |
| `deploy/` | Instalação do relay numa EC2: Redis só em localhost com ACL, serviço systemd isolado, TLS automático | ✅ Usado no deploy atual |

## O relay em números

O relay é pequeno de propósito: só passa mensagens cifradas, não guarda histórico (as imagens cifradas ficam lá no máximo 24 horas) e a criptografia pesada corre nos telemóveis. Medição com 10 000 ligações abertas em simultâneo, cada uma a pedir mensagens de 3 em 3 segundos, como as apps fazem enquanto estão abertas:

| | Resultado |
|---|---|
| Ligações abertas ao mesmo tempo | **10 000 de 10 000, 0 falhas** |
| Memória do relay | **138 MB no total, cerca de 14 KB por ligação** (arranca com 4 MB) |
| Pedidos respondidos | **~3 300 por segundo, 0 erros** |
| CPU por pedido | cerca de 0,05 ms no relay e 0,02 ms no Redis (24 % de um núcleo para as 10 000 ligações) |
| CPU com as ligações paradas | 0 % |

Com mensagens sempre a circular (50 pares de ligações a enviar o mais depressa possível), o relay entregou cerca de **16 900 mensagens de 200 B por segundo** e **6 000 de 4 KB**, com 95 % das mensagens a chegar ao destinatário em menos de 10 ms (200 B) e 26 ms (4 KB). Uma imagem de 25 MB foi enviada ao relay em 0,3 s e descarregada em 0,4 s.

Medido numa máquina local de 2 vCPU, sem o TLS do proxy e sem rede real pelo meio, por isso o servidor completo gasta mais do que isto. Como foi medido, como repetir e o que ficou de fora: [`docs/performance.md`](docs/performance.md).

## Tempos reais, entre telemóveis

Os números de cima são do relay sozinho. Com telemóveis reais (três Samsung, em Viseu e em Cascais, em Wi-Fi e em dados móveis, com o relay em Estocolmo), medidos pelo próprio relay a 19 de outubro de 2026, numa hora com 110 mensagens:

| | Mensagem típica (mediana) | 95 % das mensagens |
|---|---|---|
| Do telemóvel de quem envia até ao servidor | **42 ms** | 57 ms |
| Do servidor até o outro telemóvel ter a mensagem | **141 ms** | 18 s |

As apps ficam à espera numa ligação aberta e o relay entrega cada mensagem no instante em que ela chega. As poucas mensagens lentas são para telemóveis que estavam a dormir: a notificação acorda-os e a mensagem chega quando a app abre.

## Chamadas de voz

**As chamadas de voz a dois funcionam na app Android**, testadas a 19 de outubro de 2026 entre dois telemóveis, com muito boa qualidade. Desde a 0.9.6 tocam com a app fechada ou o telemóvel bloqueado; desde a 0.9.8 com um toque próprio da SEALER. O iPhone vem depois.

<p align="center"><img src="docs/img/chamada-0.9.4.jpg" width="260" alt="Ecrã de uma chamada de voz na SEALER 0.9.4: contador, cifrada ponta a ponta, ligação por UDP a 64 ms do servidor, e os botões Silenciar, Altifalante e Desligar."></p>

| Fase | O que é | Estado |
|---|---|---|
| 1. Sinalização e cifra | Ligar, tocar, atender e desligar vão como mensagens cifradas dentro do Double Ratchet. Cada lado contribui com um segredo aleatório, e daí saem as chaves do áudio | ✅ Feita (`core/src/call.rs`, `ffi/src/call.rs`) |
| 2. Relay das chamadas | Serviço à parte no servidor (UDP 443) que só reencaminha pacotes de áudio cifrados | ✅ Em produção, testado a partir de casa |
| 3. Motor de áudio | Opus 1.6 com DRED e PLC neural, jitter buffer adaptativo e ligação ao relay | ✅ Feita e testada em Rust (`call/`) |
| 4a. App Android, com a app aberta | Botão de ligar, ecrã de chamada, microfone com o cancelamento de eco do telemóvel, altifalante e silenciar | ✅ Feita (Android 0.9.4), testada em telemóveis reais |
| 4b. Tocar com a app fechada | Aviso de chamada com a app fechada ou o telemóvel bloqueado, com Atender e Recusar por cima do bloqueio | ✅ Feita (Android 0.9.6) |
| 5. iPhone e sistema de chamadas | Chamadas no iPhone; no Android, auricular Bluetooth, carro e chamada normal ao mesmo tempo | ⏳ A seguir |

**Como protege a privacidade:**

- **O áudio passa sempre pelo servidor e nunca vai direto entre telemóveis.** Assim, quem te liga nunca fica a saber o teu IP, nem tu o dele.
- **O servidor não sabe quem está a falar com quem.** Cada telemóvel entra numa "sala" com um pedido cifrado para a chave do servidor, sem dizer qual é a conta. O servidor só vê dois endereços a trocar pacotes cifrados que não consegue abrir.
- **Todos os pacotes têm o mesmo tamanho e o mesmo ritmo** (um a cada 20 ms, também em silêncio). Com tamanhos variáveis, já foi demonstrado que se consegue adivinhar frases só pelo tamanho dos pacotes, mesmo cifrados.
- **A chamada começa como uma mensagem normal**, por isso o servidor nem sabe que é uma chamada, e a cifra herda a proteção pós-quântica do PQXDH.
- **Sem registos:** o servidor das chamadas não guarda IPs nem horas, só contagens por minuto. O limite que fica: enquanto a chamada dura, o servidor podia, em teoria, cruzar o IP da sala com o IP de quem está ligado para receber mensagens.

**Como lida com redes más:**

- **DRED (Opus 1.6):** cada pacote leva uma versão muito comprimida dos cerca de 200 ms anteriores. Quando se perdem pacotes, o seguinte reconstrói o que faltou. Num teste com 15% dos pacotes perdidos, foram todos reconstruídos.
- **PLC neural:** quando não há nada para reconstruir, um modelo preenche o buraco em vez de um corte seco.
- **Jitter buffer adaptativo:** guarda entre 40 e 200 ms de áudio, conforme a variação da rede.
- **Passar do Wi-Fi para os dados móveis não corta a chamada**, e há uma reserva por `wss://relay.sealer.pt/call` para redes que bloqueiam UDP.

**Medido a 19 de outubro de 2026**, a partir de casa, em Portugal, até ao servidor em Estocolmo: entrar na sala em 66 ms e ida e volta PC → servidor → PC → servidor → PC em 128 ms, o que dá cerca de **64 ms de rede num sentido** entre dois telemóveis. Cada chamada gasta cerca de 35 MB por hora em cada sentido. Os formatos estão na secção 18 de [`docs/protocol-spec.md`](docs/protocol-spec.md).

## O que falta, sem rodeios

Para além da lista acima ("O que ainda falta"), a mais longo prazo:

- Vários servidores independentes a colaborar entre si (federação), para que a falha de um servidor não pare a rede inteira
- Uma auditoria de segurança independente, feita por alguém que não sou eu

## Experimenta tu mesmo

A forma mais rápida de veres isto a funcionar é este comando, que mostra passo a passo uma conversa cifrada completa entre duas pessoas fictícias:

```bash
cargo run -p qt_cli --bin messenger_demo
```

### Requisitos para compilar

```bash
sudo apt install redis-server libsqlcipher-dev cmake
redis-server --daemonize yes
```
Rust 1.82 ou mais recente, via [rustup](https://rustup.rs). O `cmake` compila o Opus das chamadas de voz.

### Correr os testes

```bash
cargo test --workspace                          # tudo, incluindo o pós-quântico (precisa do Redis acima)
cargo test -p qt_core --features pq              # só o núcleo criptográfico
```

O teste `server/tests/redis_acl.rs` arranca um Redis próprio com a lista de comandos de produção; sem `redis-server` instalado é saltado.

### Duas pessoas a falar através do relay (no PC)

```bash
cargo run --release -p qt_server --bin relay_server &       # relay local em 127.0.0.1:9443

# em dois terminais diferentes (cada um e uma "pessoa"):
cargo run -p qt_server --example peer -- --relay ws://127.0.0.1:9443 --db alice.db
cargo run -p qt_server --example peer -- --relay ws://127.0.0.1:9443 --db bob.db
# em cada um: /id mostra o Public ID; /add <ID do outro>; depois escreve mensagens
```

O cliente `peer` usa exatamente o mesmo protocolo da app Android. O identificador de cada pessoa nasce da sua chave pública: ninguém escolhe um "nome de utilizador", de propósito. `ws://` só é aceite para `localhost` em builds de debug; fora disso o transporte exige `wss://`.

### O mensageiro no terminal (fala com os telemóveis)

O `messenger` usa o mesmo núcleo e o mesmo relay que as apps: o que escreves no PC chega ao telemóvel, e o contrário.

```bash
cargo build --release -p qt_cli --bin messenger
export QT_RELAY_URL=wss://<o-teu-relay>  QT_RELAY_PIN=<chave publica Noise do relay>
M=./target/release/messenger

$M identity --db eu.sqlite                 # uma vez: cria a identidade e mostra o teu QT Public ID
$M add --db eu.sqlite Ana qt1:...          # guarda um contacto (o ID que a app dele mostra)
$M send --db eu.sqlite --to Ana            # a mensagem e pedida no terminal
$M listen --db eu.sqlite                   # fica a receber (Ctrl+C para sair)
$M history --db eu.sqlite Ana              # historico, guardado cifrado
$M remove --db eu.sqlite Ana               # apaga o contacto, as mensagens e as chaves da conversa
```

A passphrase é sempre pedida no terminal, escondida (ou lida de uma linha do stdin com `--passphrase-stdin`, para scripts). Dela saem, por Argon2id, a chave da base de dados e a chave da lista de contactos, que também fica cifrada.

### Ouvir uma chamada no PC (eco)

A ferramenta `tools/call-demo` usa o mesmo motor das chamadas: a tua voz vai ao servidor das chamadas e volta, e ouves-te com o atraso real da rede, do jitter buffer e do Opus. Usa auscultadores (no PC não há cancelamento de eco).

```bash
sudo apt install cmake libasound2-dev
cd tools/call-demo
cargo run --release -- eco              # microfone -> servidor -> auscultadores
cargo run --release -- eco --perda 15   # deita fora 15% dos pacotes: ouve o DRED a reconstruir
cargo run --release -- eco --ws         # usa a reserva por WebSocket
```

### Gerar os bindings para Android/iOS/Python

```bash
./ffi/generate_bindings.sh
```

## App Android

O core Rust é o único backend criptográfico da app Android (Kotlin/Compose): API `QtClient` via UniFFI, storage SQLCipher com chave do Android Keystore, PQXDH (X25519 + ML-KEM-768) + Double Ratchet + Sealed Sender, relay autenticado com entrega *at-least-once*, notificações push (FCM, aviso genérico), chamadas de voz (motor `call/`) e textos em seis línguas (`res/values*/strings.xml`). **Versão de testes (0.9.11), não auditada.**

```bash
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/<versao>
ABIS="arm64-v8a armeabi-v7a x86_64" ./scripts/build-android.sh   # libqt_ffi.so (16 KB) + bindings Kotlin
cd android && ./sync-native.sh                                    # copia para a app
# depois: abrir a pasta android/ no Android Studio
```

A build *release* exige uma keystore própria e o relay de produção (`qtRelayUrl` / `qtRelayPin` em `android/gradle.properties`); nessa build o relay é fixo e não aparece na interface. Usa o NDK 30 (o r26d não compila o OpenSSL incluído nem a `x86_64` do emulador). Passo a passo em [`android/README.md`](android/README.md).

* Guia de integração e build: [`docs/android-integration.md`](docs/android-integration.md)
* Referência da API: [`docs/ffi-api.md`](docs/ffi-api.md)
* Protocolo (§8–18 = formatos atuais, incluindo as chamadas): [`docs/protocol-spec.md`](docs/protocol-spec.md)
* Ameaças e limitações: [`docs/threat-model.md`](docs/threat-model.md), [`docs/known-limitations.md`](docs/known-limitations.md)

## App iPhone

Precisa de um Mac com Xcode e do [XcodeGen](https://github.com/yonaskolb/XcodeGen):

```bash
./scripts/build-ios.sh          # compila o núcleo Rust para iPhone e simulador
cd ios && xcodegen && open SealerMessenger.xcodeproj
```

Passo a passo, contas Apple e diferenças em relação ao Android em [`ios/README.md`](ios/README.md).

## Relay na AWS

Numa EC2 nova com Ubuntu 24.04 (Security Group só com 22, 80 e 443 TCP, e 443 UDP para as chamadas):

```bash
curl -fsSLO https://raw.githubusercontent.com/moral666/QT/site-sealer/deploy/aws/setup-ec2.sh
sudo bash setup-ec2.sh
```

Instala o Redis (só `127.0.0.1`, utilizador com ACL mínima), o relay como serviço systemd isolado, o relay das chamadas de voz (UDP 443, num serviço à parte sem acesso ao Redis) e o HAProxy com certificado TLS (certbot, renovação automática), e imprime o URL `wss://` e a chave de pinning para a app. As imagens cifradas ficam em `/var/lib/qt/anexos` (no máximo 2 GB no total e 24 horas cada, por omissão).

Para atualizar um relay já instalado, corre o mesmo script: vai buscar o código novo e recompila, mas mantém a chave Noise (o pin das apps não muda) e a password do Redis. A compilação é feita na própria instância e precisa de espaço: num disco de 8 GB ficou quase cheio, o relay atual usa 20 GB. Detalhes em [`docs/deploy-wss.md`](docs/deploy-wss.md).

## Como o projeto está organizado

```
core/        O coração: cifra ponta-a-ponta, sem depender de mais nada
transport/   Protege a ligação em rede
server/      Entrega mensagens sem as poder ler
storage/     Guarda tudo em disco, cifrado
cli/         O mensageiro no terminal (mesmo núcleo e relay que as apps)
demo/        Junta core+transport num único teste, para ver tudo em ação
ffi/         A ponte para Kotlin, Swift e Python
call/        O motor das chamadas de voz (Opus, jitter buffer, rede)
android/     A app Android (Kotlin/Compose)
ios/         A app iPhone (SwiftUI)
branding/    Logotipo, ícones e fonte da marca SEALER
site/        O site sealer.pt (página estática na Cloudflare)
deploy/      Instalação do relay em servidor (AWS, Redis, systemd)
scripts/     Compilação para Android e iPhone, e medições (relay e telemóvel)
tools/       Ferramentas de teste (eco das chamadas de voz no PC)
docs/        A especificação técnica completa, e o histórico de decisões
```

Cada peça tem os seus próprios testes, guardados ao lado do código (`tests/`).

## Queres perceber os detalhes técnicos?

- [`docs/protocol-spec.md`](docs/protocol-spec.md) explica, secção a secção, exatamente como cada peça funciona por dentro: que algoritmos, que decisões e o que ainda falta em cada uma.
- [`docs/threat-model.md`](docs/threat-model.md) explica contra quem e contra o quê isto protege e, tão importante quanto isso, contra quem e contra o quê **não** protege. Leitura recomendada antes de confiares em qualquer alegação de segurança feita aqui.
- [`docs/known-limitations.md`](docs/known-limitations.md) lista as limitações conhecidas, por ordem de impacto.
- [`docs/field-tests.md`](docs/field-tests.md) e [`docs/performance.md`](docs/performance.md) registam os testes com aparelhos reais e as medições do relay.
- [`ROADMAP.md`](ROADMAP.md) mostra o que falta, por ordem de prioridade.
- [`CHANGELOG.md`](CHANGELOG.md) mostra o histórico do que já mudou.

## Licença

QT é disponibilizado sob uma licença dual:

- **Open source:** GNU Affero General Public License, versão 3 ou posterior (AGPL-3.0-or-later).
- **Comercial:** organizações que necessitem de termos diferentes da AGPL podem obter uma licença comercial mediante acordo escrito com o autor.

A licença comercial permite definir contratualmente termos específicos de utilização, integração, distribuição, deployment, suporte, manutenção e outros direitos comerciais.

Os componentes e dependências de terceiros utilizados pelo QT permanecem sujeitos às respetivas licenças.

Ver [`LICENSE`](LICENSE) para a licença AGPL e [`LICENSE-COMMERCIAL.md`](LICENSE-COMMERCIAL.md) para informação sobre o modelo de licenciamento comercial.

## Queres ajudar ou reportar um problema?

- `CONTRIBUTING.md` explica como propor mudanças, e tem regras extra (razoáveis) para quem mexer em código de criptografia
- `SECURITY.md` explica como reportar uma vulnerabilidade em privado. Por favor não abras uma issue pública para isso
