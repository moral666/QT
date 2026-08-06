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

Se preferires uma explicação totalmente sem jargão técnico, começa por [`apresentacao-projeto.md`](../apresentacao-projeto.md).

## Sê honesto comigo: isto já funciona?

Sim e não — e vale a pena seres claro sobre isto antes de investires tempo a explorar o projeto.

**Já funciona e já foi testado a sério** (não é só código que "devia funcionar"): a cifra ponta-a-ponta, a camada de rede, um servidor que entrega mensagens sem nunca conseguir lê-las, uma app Android que já correu num telemóvel real. Tudo isto tem testes automáticos a passar, e várias partes foram testadas manualmente da forma mais rigorosa que consegui (por exemplo: matar o servidor a meio e confirmar que os dados sobrevivem).

**Ainda não dá para usar no dia a dia**: falta a interface de utilizador completa (hoje a app Android só tem um botão de "correr demonstração"), falta pôr o servidor num sítio acessível pela internet, e falta uma auditoria de segurança externa antes de qualquer pessoa confiar nisto com conversas reais.

Por outras palavras: as fundações estão sólidas e testadas; o prédio ainda não tem telhado.

## O que já está construído

| Peça | O que faz | Confiança |
|---|---|---|
| `core/` | Estabelecimento de sessão (X3DH + variante pós-quântica) e cifra contínua (Double Ratchet) — o mesmo desenho usado pelo Signal | ✅ Testado (14+ testes automáticos, incluindo casos que devem falhar propositadamente) |
| `transport/` | Protecção da ligação em rede (Noise Protocol) sobre WebSocket | ✅ Testado com ligações WebSocket reais, não simuladas |
| `server/` | O "carteiro": entrega mensagens sem nunca ver o conteúdo nem saber quem as enviou | ✅ Testado, incluindo sobreviver a ser desligado à força (`kill -9`) |
| `storage/` | Guarda a tua identidade e conversas no disco, cifradas | ✅ Testado, incluindo confirmar que uma password errada não abre nada |
| `cli/` | Uma versão de terminal, para experimentar sem precisar de telemóvel | ✅ Testado com processos completamente separados a falar entre si |
| `ffi/` | A ponte que liga tudo isto a Kotlin (Android), Swift (iOS) e Python | ✅ Testado a partir do Python |
| App Android | Usa essa ponte para provar que tudo funciona num telemóvel real | ✅ Já correu num emulador Android real |

## O que falta, sem rodeios

- Um ecrã de conversa a sério na app (hoje é só um botão de demonstração)
- Um servidor acessível pela internet, não só na tua rede local
- Ligação protegida por TLS (`wss://`) — hoje usa `ws://` simples, adequado só para testes
- Vários servidores independentes a colaborar entre si (federação), para que nenhum governo consiga desligar a rede inteira desligando só um
- A password da base de dados vir do sistema de segurança do telemóvel (Keystore/Secure Enclave), não de um argumento de linha de comandos
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

### Usar como cliente persistente (duas pessoas, processos separados)

```bash
cargo run --bin relay_server &

cargo run -p qt_cli --bin messenger -- identity --db bob.sqlite --passphrase "..."
cargo run -p qt_cli --bin messenger -- register --db bob.sqlite --passphrase "..." --server ws://127.0.0.1:9443

cargo run -p qt_cli --bin messenger -- identity --db alice.sqlite --passphrase "..."
cargo run -p qt_cli --bin messenger -- send --db alice.sqlite --passphrase "..." --to <ID-DO-BOB> --message "Ola!" --server ws://127.0.0.1:9443

cargo run -p qt_cli --bin messenger -- receive --db bob.sqlite --passphrase "..." --server ws://127.0.0.1:9443
```

O identificador de cada pessoa nasce da sua chave pública — ninguém escolhe um "nome de utilizador", de propósito.

### Gerar os bindings para Android/iOS/Python

```bash
./ffi/generate_bindings.sh
```

## Como o projeto está organizado

```
core/        O coração: cifra ponta-a-ponta, sem depender de mais nada
transport/   Protege a ligação em rede
server/      Entrega mensagens sem as poder ler
storage/     Guarda tudo em disco, cifrado
cli/         Um cliente de terminal, para testar sem telemóvel
demo/        Junta core+transport num único teste, para ver tudo em ação
ffi/         A ponte para Kotlin, Swift e Python
docs/        A especificação técnica completa, e o histórico de decisões
```

Cada peça tem os seus próprios testes, guardados ao lado do código (`tests/`). A app Android vive por agora só na máquina onde foi feita — quando for publicada, deve ganhar o seu próprio repositório, que usa este aqui como dependência (é o mesmo princípio de separação explicado em `CONTRIBUTING.md`).

## Queres perceber os detalhes técnicos?

`docs/protocol-spec.md` explica, secção a secção, exatamente como cada peça funciona por dentro — que algoritmos, que decisões, e o que ainda falta em cada uma. É o documento certo se quiseres avaliar isto tecnicamente ou contribuir.

## Licença

AGPL-3.0-or-later. Em linguagem simples: qualquer pessoa pode usar, estudar e modificar este código livremente — mas se alguém pegar nele para oferecer um serviço (mesmo só através da internet, sem "distribuir" software no sentido tradicional), é obrigado a partilhar também o código dessas modificações. Isto fecha uma porta que licenças mais permissivas deixam aberta: a de alguém pegar num projeto aberto e transformá-lo num serviço fechado.

## Queres ajudar ou reportar um problema?

- `CONTRIBUTING.md` explica como propor mudanças, e tem regras extra (razoáveis) para quem mexer em código de criptografia
- `SECURITY.md` explica como reportar uma vulnerabilidade em privado — por favor não abras uma issue pública para isso
