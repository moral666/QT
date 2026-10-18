# QT

<p align="center"><img src="branding/sealer-logo-transparente.png" width="140" alt="Logotipo SEALER"></p>

As apps chamam-se **SEALER**; QT é o nome do protocolo e deste repositório.

**Versão alpha, não auditada.** Um mensageiro E2EE com criptografia
pós-quântica (X3DH, PQXDH, Double Ratchet, Sealed Sender, Noise Protocol),
tudo implementado em Rust, com apps Android e iPhone e relay na nuvem, já
a funcionar de ponta a ponta. **Ainda não é um produto acabado nem deve ser
usado para informação sensível.** Ver a secção "Sê honesto comigo: isto
já funciona?" já a seguir.

[Read this in English →](README.en.md)

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

Sim, de ponta a ponta em versão alpha. Não foi auditado de forma independente e não deve ser usado para conversas reais ou informação sensível.

### Validado até 15 de outubro de 2026

- Núcleo criptográfico (PQXDH + Double Ratchet + Sealed Sender), transporte, relay, armazenamento local e FFI implementados em Rust, com a suite de testes a passar (`cargo test --workspace`).
- Relay a correr numa instância AWS EC2, atrás de TLS real (`wss://`, certificado Let's Encrypt via certbot e HAProxy), com o Redis acessível apenas em `127.0.0.1` e protegido por ACL.
- App Android compilada em versão *release* assinada (esquemas V2/V3), bibliotecas nativas alinhadas a páginas de 16 KB, sem deteções no VirusTotal (0/68).
- Mensagens trocadas entre dois telemóveis reais, através do relay na AWS.
- App iPhone (SwiftUI) com o mesmo núcleo, a trocar mensagens com as apps Android.
- Teste de campo com cinco aparelhos em três plataformas (2 Android reais, 1 emulador Android, 1 iPhone e 1 Mac), com envios simultâneos e contínuos: nenhuma mensagem perdida. Detalhes, problemas encontrados e o que ficou por testar em [`docs/field-tests.md`](docs/field-tests.md).
- Relay com 10 000 ligações em simultâneo (cada uma a pedir mensagens de 3 em 3 segundos, como as apps): nenhuma falha, cerca de 14 KB de memória por ligação. Ver a secção "O relay em números" mais abaixo.

### Novo a 16 e 17 de outubro de 2026

- **Entrega imediata, testada com telemóveis reais:** o tempo típico até a mensagem chegar ao outro telemóvel passou de **cerca de 2 000 ms para 300 a 430 ms**. Ver "Tempos reais, entre telemóveis" mais abaixo.
- **Apagar a conta também no servidor:** "Destruir perfil e dados" apaga as chaves públicas e as mensagens em espera no relay, e depois tudo do telemóvel.
- **App Android mais leve na bateria:** 30 segundos depois de sair da app, fecha as ligações ao relay e volta a ligar quando a abres.
- **Preparação para a Google Play:** política de privacidade e página para apagar a conta no sealer.pt, e respostas para os formulários da Play Console em [`docs/google-play.md`](docs/google-play.md).

### Acabado de juntar, ainda por confirmar no uso real

- **Imagens e GIFs até 25 MB**, nas duas apps. Cada ficheiro é cifrado no telemóvel com uma chave própria, segue para o relay em blocos e é apagado de lá quando o outro telemóvel o descarrega, ou ao fim de 24 horas. Testado nos testes automáticos (incluindo com a lista de comandos do Redis de produção); a troca entre telemóveis através da AWS ainda está a ser confirmada. Detalhes em [`docs/protocol-spec.md`](docs/protocol-spec.md), secção 14.
- **Adicionar contactos por código QR**, nas duas apps (o código é gerado e lido no telemóvel, sem rede).
- Marca SEALER nas apps (logotipo, nome e tema).

A app iPhone com estas novidades ainda não foi compilada no Xcode (ver [`ios/README.md`](ios/README.md)).

### O que ainda falta

- ~~Histórico persistente~~ (feito): histórico guardado no SQLCipher (na mesma transação que o avanço do ratchet), com indicadores *Enviado / Entregue / Visto* através de recibos cifrados e fila de saída persistente.
- ~~Adição de contactos por código QR~~ (feito nas duas apps).
- **Notificações push**: a app só recebe mensagens enquanto está aberta.
- **Infraestrutura de produção**: domínio próprio (o atual é provisório), redundância, backups e alertas do servidor.
- ~~Bloqueio da app~~ (feito no Android): código de acesso próprio, que cifra a chave das conversas (Argon2id), e desbloqueio opcional com impressão digital, rosto ou PIN do telemóvel. Falta no iPhone.
- Distribuição pela Play Store (hoje alguns antivírus avisam por ser uma app de origem desconhecida). Já preparado: eliminação de conta, política de privacidade e respostas dos formulários ([`docs/google-play.md`](docs/google-play.md)).
- Teste de carga no relay da AWS (já medido numa máquina local, ver [`docs/performance.md`](docs/performance.md)) e **auditoria de segurança independente**.

**Próximas etapas:** *alpha* (agora: funciona de ponta a ponta, testado com poucas pessoas) → *beta* (notificações, bloqueio da app no iPhone, teste na Play Store) → *1.0* (depois de auditoria externa).

## Arquitetura

<!-- Gerado por site/scripts/architecture_diagram.py (ver site/README.md, secção 14). -->
![Arquitetura da SEALER: as apps de Android e iPhone usam o mesmo núcleo QT em Rust e trocam envelopes cifrados através de um relay na AWS (Caddy, relay em Rust e Redis). O site sealer.pt está à parte, na Cloudflare.](docs/arquitetura.svg)

## O que já está construído

| Peça | O que faz | Confiança |
|---|---|---|
| `core/` | Estabelecimento de sessão (X3DH + variante pós-quântica) e cifra contínua (Double Ratchet), o mesmo desenho usado pelo Signal | ✅ Testado (mais de 40 testes automáticos, incluindo casos que devem falhar propositadamente) |
| `transport/` | Proteção da ligação em rede (Noise Protocol) sobre WebSocket | ✅ Testado com ligações WebSocket reais, não simuladas |
| `server/` | O "carteiro": entrega mensagens sem nunca ver o conteúdo nem saber quem as enviou, e guarda as imagens cifradas no máximo 24 horas | ✅ Testado, incluindo sobreviver a ser desligado à força (`kill -9`) e com a lista de comandos do Redis de produção |
| `storage/` | Guarda a tua identidade e conversas no disco, cifradas | ✅ Testado, incluindo confirmar que uma password errada não abre nada |
| `cli/` | O mensageiro no terminal (`messenger`), com o mesmo núcleo das apps, e uma demonstração visível do protocolo (`messenger_demo`) | ✅ Testado de ponta a ponta com um relay real: terminal ↔ terminal e terminal ↔ cliente igual ao das apps |
| `ffi/` | A ponte que liga tudo isto a Kotlin (Android), Swift (iOS) e Python: histórico, fila de saída, imagens, bloqueio da app | ✅ Testado em Rust (incluindo sessões cruzadas e imagens) e a partir do Python |
| `android/` | App Android (Kotlin/Compose); toda a criptografia continua no Rust | ✅ Release assinada testada entre dois telemóveis reais através do relay na AWS |
| `ios/` | App iPhone (SwiftUI), igual à Android, com o mesmo núcleo Rust | ✅ Compilada num Mac e a trocar mensagens através do relay na AWS (as novidades mais recentes ainda por compilar) |
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

Os números de cima são do relay sozinho. Com telemóveis reais (três Samsung, em Viseu e em Cascais, em Wi-Fi e em dados móveis, com o relay em Estocolmo), medidos pelo próprio relay em outubro de 2026, com centenas de mensagens:

| | Antes (16/10) | Agora (17/10) |
|---|---|---|
| Do servidor até o outro telemóvel ter a mensagem (tempo típico, mediana) | **cerca de 2 000 ms** | **300 a 430 ms** |
| Do telemóvel de quem envia até ao servidor (média) | | **cerca de 50 ms** |

A grande diferença veio da **entrega imediata**: antes, as apps perguntavam ao relay de 3 em 3 segundos se havia mensagens novas; agora ficam à espera numa ligação aberta e o relay entrega cada mensagem no instante em que ela chega. O que sobra é sobretudo o caminho pela rede até à Suécia e de volta, e o telemóvel a decifrar e guardar a mensagem. Com o relay mais perto (por exemplo, em Espanha), estes tempos devem descer mais.

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
sudo apt install redis-server libsqlcipher-dev
redis-server --daemonize yes
```
Rust 1.81 ou mais recente, via [rustup](https://rustup.rs) (a parte pós-quântica exige-o).

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

### Gerar os bindings para Android/iOS/Python

```bash
./ffi/generate_bindings.sh
```

## App Android

O core Rust é o único backend criptográfico da app Android (Kotlin/Compose): API `QtClient` via UniFFI, storage SQLCipher com chave do Android Keystore, PQXDH (X25519 + ML-KEM-768) + Double Ratchet + Sealed Sender, relay autenticado com entrega *at-least-once*. **Versão alpha, não auditada.**

```bash
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/<versao>
ABIS="arm64-v8a armeabi-v7a" ./scripts/build-android.sh   # libqt_ffi.so (16 KB) + bindings Kotlin
cd android && ./sync-native.sh ..                          # copia para a app
# depois: abrir a pasta android/ no Android Studio
```

A build *release* exige uma keystore própria e o relay de produção (`qtRelayUrl` / `qtRelayPin` em `android/gradle.properties`); nessa build o relay é fixo e não aparece na interface. Usa o NDK 27 ou mais recente (com o r26d a compilação do OpenSSL incluído falhou). Passo a passo em [`android/README.md`](android/README.md).

* Guia de integração e build: [`docs/android-integration.md`](docs/android-integration.md)
* Referência da API: [`docs/ffi-api.md`](docs/ffi-api.md)
* Protocolo (§8–14 = formatos atuais): [`docs/protocol-spec.md`](docs/protocol-spec.md)
* Ameaças e limitações: [`docs/threat-model.md`](docs/threat-model.md), [`docs/known-limitations.md`](docs/known-limitations.md)

## App iPhone

Precisa de um Mac com Xcode e do [XcodeGen](https://github.com/yonaskolb/XcodeGen):

```bash
./scripts/build-ios.sh          # compila o núcleo Rust para iPhone e simulador
cd ios && xcodegen && open SealerMessenger.xcodeproj
```

Passo a passo, contas Apple e diferenças em relação ao Android em [`ios/README.md`](ios/README.md).

## Relay na AWS

Numa EC2 nova com Ubuntu 24.04 (Security Group só com 22, 80 e 443):

```bash
curl -fsSLO https://raw.githubusercontent.com/moral666/QT/security-hardening/deploy/aws/setup-ec2.sh
sudo bash setup-ec2.sh
```

Instala o Redis (só `127.0.0.1`, utilizador com ACL mínima), o relay como serviço systemd isolado e o HAProxy com certificado TLS (certbot, renovação automática), e imprime o URL `wss://` e a chave de pinning para a app. As imagens cifradas ficam em `/var/lib/qt/anexos` (no máximo 2 GB no total e 24 horas cada, por omissão).

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
android/     A app Android (Kotlin/Compose)
ios/         A app iPhone (SwiftUI)
branding/    Logotipo, ícones e fonte da marca SEALER
site/        O site sealer.pt (página estática na Cloudflare)
deploy/      Instalação do relay em servidor (AWS, Redis, systemd)
scripts/     Compilação para Android e iPhone, e medições (relay e telemóvel)
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
