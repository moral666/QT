# QT

**Um protótipo técnico** do núcleo criptográfico e de transporte de um
mensageiro E2EE — X3DH, PQXDH (pós-quântico), Double Ratchet, Sealed
Sender, Noise Protocol, tudo implementado em Rust, compilado e testado a
sério. **Não é um produto acabado nem pronto para conversas reais** — ver
a secção "Sê honesto comigo: isto já funciona?" já a seguir, antes de
investires tempo a explorar isto.

[Read this in English →](README.en.md)

## Contexto

Este projeto nasceu de um interesse genuíno em privacidade e segurança de
sistemas — em particular, na questão de como construir infraestrutura de
comunicação que seja resistente a backdoors por desenho, não só por
política (motivada, entre outras coisas, por leis como o "Chat Control"
discutido na União Europeia). Em vez de ficar só na teoria, o objetivo foi
implementar e testar cada peça a sério: não pseudocódigo, não diagramas —
código que compila, corre, e tem testes automáticos a confirmar que faz o
que diz que faz.

## Porque é que isto existe (o objetivo, para além do exercício técnico)

Quase todos os mensageiros populares pedem o teu número de telefone para te registares. Mesmo quando o conteúdo das mensagens é privado, esse registo cria um rasto — quem fala com quem, quando, com que frequência. Há também leis a serem discutidas na União Europeia (o chamado "Chat Control") que podem obrigar aplicações a vasculhar o conteúdo das mensagens das pessoas, mesmo em apps que se dizem "cifradas".

O QT tenta responder a isto com engenharia, não só com uma política de privacidade: identidade baseada numa chave criptográfica (sem telefone, sem email), cifra ponta-a-ponta que nem o servidor consegue quebrar, e código aberto para que qualquer pessoa possa verificar que não há truques escondidos.

## Sê honesto comigo: isto já funciona?

Sim, de ponta a ponta — como protótipo. **Não foi auditado de forma independente e não deve ser usado para conversas reais ou informação sensível.**

### Validado até 14 de outubro de 2026

- Núcleo criptográfico (PQXDH + Double Ratchet + Sealed Sender), transporte, relay, armazenamento local e FFI implementados em Rust, com a suite de testes a passar (`cargo test --workspace`).
- Relay a correr numa instância AWS EC2, atrás de TLS real (`wss://`, certificado Let's Encrypt via Caddy), com o Redis acessível apenas em `127.0.0.1` e protegido por ACL.
- App Android compilada em versão *release* assinada (esquemas V2/V3), bibliotecas nativas alinhadas a páginas de 16 KB, sem deteções no VirusTotal (0/68).
- **Mensagens trocadas entre dois telemóveis reais, através do relay na AWS.**

### O que ainda falta

- **Histórico de conversas persistente** — hoje as mensagens recebidas só vivem em memória enquanto a app está aberta.
- **Notificações push** — a app só recebe mensagens enquanto está aberta.
- **Infraestrutura de produção** — domínio próprio (o atual é provisório), redundância, backups e alertas do servidor.
- Bloqueio da app com PIN/biometria, adição de contactos por QR code, interface mais cuidada.
- Distribuição pela Play Store (hoje alguns antivírus avisam por ser uma app de origem desconhecida).
- O cliente de terminal `messenger` ainda usa o formato antigo de registo e não funciona com o relay atual (o `messenger_demo` e o cliente de teste `peer` funcionam).
- Testes de carga e **auditoria de segurança independente**.

## O que já está construído

| Peça | O que faz | Confiança |
|---|---|---|
| `core/` | Estabelecimento de sessão (X3DH + variante pós-quântica) e cifra contínua (Double Ratchet) — o mesmo desenho usado pelo Signal | ✅ Testado (14+ testes automáticos, incluindo casos que devem falhar propositadamente) |
| `transport/` | Protecção da ligação em rede (Noise Protocol) sobre WebSocket | ✅ Testado com ligações WebSocket reais, não simuladas |
| `server/` | O "carteiro": entrega mensagens sem nunca ver o conteúdo nem saber quem as enviou | ✅ Testado, incluindo sobreviver a ser desligado à força (`kill -9`) |
| `storage/` | Guarda a tua identidade e conversas no disco, cifradas | ✅ Testado, incluindo confirmar que uma password errada não abre nada |
| `cli/` | Uma versão de terminal, para experimentar sem precisar de telemóvel | ⚠️ `messenger_demo` funciona; o cliente persistente `messenger` precisa de ser atualizado para o protocolo atual do relay |
| `ffi/` | A ponte que liga tudo isto a Kotlin (Android), Swift (iOS) e Python | ✅ Testado a partir do Python |
| `android/` | App Android (Kotlin/Compose) — toda a criptografia continua no Rust | ✅ Release assinada testada entre dois telemóveis reais através do relay na AWS |
| `deploy/` | Instalação do relay numa EC2: Redis só em localhost com ACL, serviço systemd isolado, TLS automático | ✅ Usado no deploy atual |

## O que falta, sem rodeios

Para além da lista acima ("O que ainda falta"), a mais longo prazo:

- Vários servidores independentes a colaborar entre si (federação), para que ninguém consiga desligar a rede inteira desligando só um
- App iOS (os bindings Swift já são gerados, mas nunca foram usados numa app)
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
Rust via [rustup](https://rustup.rs) — qualquer versão recente serve, exceto a variante pós-quântica do núcleo, que exige Rust 1.81 ou mais recente.

### Correr os testes

```bash
cargo test --workspace                          # tudo, exceto pós-quântico
cargo test -p qt_core --features pq              # incluindo pós-quântico
```

### Duas pessoas a falar através do relay (no PC)

```bash
cargo run --release -p qt_server --bin relay_server &       # relay local em 127.0.0.1:9443

# em dois terminais diferentes (cada um e uma "pessoa"):
cargo run -p qt_server --example peer -- --relay ws://127.0.0.1:9443 --db alice.db
cargo run -p qt_server --example peer -- --relay ws://127.0.0.1:9443 --db bob.db
# em cada um: /id mostra o Public ID; /add <ID do outro>; depois escreve mensagens
```

O cliente `peer` usa exatamente o mesmo protocolo da app Android. O identificador de cada pessoa nasce da sua chave pública — ninguém escolhe um "nome de utilizador", de propósito. `ws://` só é aceite para `localhost` em builds de debug; fora disso o transporte exige `wss://`.

### Gerar os bindings para Android/iOS/Python

```bash
./ffi/generate_bindings.sh
```

## App Android

O core Rust é o único backend criptográfico da app Android (Kotlin/Compose): API `QtClient` via UniFFI, storage SQLCipher com chave do Android Keystore, PQXDH (X25519 + ML-KEM-768) + Double Ratchet + Sealed Sender, relay autenticado com entrega *at-least-once*. **Continua a ser um protótipo não auditado.**

```bash
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/<versao>
ABIS="arm64-v8a armeabi-v7a" ./scripts/build-android.sh   # libqt_ffi.so (16 KB) + bindings Kotlin
cd android && ./sync-native.sh ..                          # copia para a app
# depois: abrir a pasta android/ no Android Studio
```

A build *release* exige uma keystore própria e o relay de produção (`qtRelayUrl` / `qtRelayPin` em `android/gradle.properties`); nessa build o relay é fixo e não aparece na interface. Passo a passo em [`android/README.md`](android/README.md).

* Guia de integração e build: [`docs/android-integration.md`](docs/android-integration.md)
* Referência da API: [`docs/ffi-api.md`](docs/ffi-api.md)
* Protocolo (§8–11 = v0.2): [`docs/protocol-spec.md`](docs/protocol-spec.md)
* Ameaças e limitações: [`docs/threat-model.md`](docs/threat-model.md), [`docs/known-limitations.md`](docs/known-limitations.md)

## Relay na AWS

Numa EC2 nova com Ubuntu 24.04 (Security Group só com 22, 80 e 443):

```bash
curl -fsSLO https://raw.githubusercontent.com/moral666/QT/security-hardening/deploy/aws/setup-ec2.sh
sudo bash setup-ec2.sh
```

Instala o Redis (só `127.0.0.1`, utilizador com ACL mínima), o relay como serviço systemd isolado e o Caddy com certificado TLS automático, e imprime o URL `wss://` e a chave de pinning para a app. Detalhes em [`docs/deploy-wss.md`](docs/deploy-wss.md).

## Como o projeto está organizado

```
core/        O coração: cifra ponta-a-ponta, sem depender de mais nada
transport/   Protege a ligação em rede
server/      Entrega mensagens sem as poder ler
storage/     Guarda tudo em disco, cifrado
cli/         Um cliente de terminal, para testar sem telemóvel
demo/        Junta core+transport num único teste, para ver tudo em ação
ffi/         A ponte para Kotlin, Swift e Python
android/     A app Android (Kotlin/Compose)
deploy/      Instalação do relay em servidor (AWS, Redis, systemd)
scripts/     Compilação das bibliotecas nativas para Android
docs/        A especificação técnica completa, e o histórico de decisões
```

Cada peça tem os seus próprios testes, guardados ao lado do código (`tests/`).

## Queres perceber os detalhes técnicos?

- [`docs/protocol-spec.md`](docs/protocol-spec.md) explica, secção a secção, exatamente como cada peça funciona por dentro — que algoritmos, que decisões, e o que ainda falta em cada uma.
- [`docs/threat-model.md`](docs/threat-model.md) explica contra quem e contra o quê isto protege — e, tão importante quanto isso, contra quem e contra o quê **não** protege. Leitura recomendada antes de confiares em qualquer alegação de segurança feita aqui.
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
- `SECURITY.md` explica como reportar uma vulnerabilidade em privado — por favor não abras uma issue pública para isso
