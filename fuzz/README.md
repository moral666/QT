# Fuzzing (local)

Alvos `cargo-fuzz` para tudo o que chega de fora: pedidos ao relay, bundles, estado do
ratchet guardado, envelopes e conteúdo das mensagens. Não corre na CI (precisa de muito
tempo de CPU); corre-se no PC antes de uma versão.

| Alvo | O que testa |
|---|---|
| `relay_pedido` | JSON que o relay recebe de qualquer cliente; verificação do bundle no registo |
| `bundle` | bundle v3/v4 e resposta `PreKeyBundle` como a app os lê; chaves e ciphertext ML-KEM |
| `estado_ratchet` | estado do ratchet lido do disco: recusado, ou lido e escrito igual |
| `envelope` | envelopes selados e respostas do relay em bruto (`receive_sealed`, `decrypt_sealed`) |
| `conteudo` | conteúdo das mensagens já decifrado (todos os tipos do payload v1), numa sessão real |

Instalar (uma vez): `cargo install cargo-fuzz --locked` e o Rust nightly
(`rustup toolchain install nightly --profile minimal`). Correr um alvo 5 minutos:

```
cd fuzz
cargo +nightly fuzz run -s none conteudo corpus/conteudo sementes/conteudo -- -max_total_time=300
```

Sem o nightly, o Rust stable também serve com `RUSTC_BOOTSTRAP=1 cargo fuzz run -s none ...`.
Um erro encontrado fica em `artifacts/<alvo>/`; repete-se com `cargo fuzz run <alvo> <ficheiro>`.
`corpus/` (gerado) e `artifacts/` não vão para o git; `sementes/` sim.

Primeira volta (22/10/2026, 2 núcleos, 4 a 7 minutos por alvo): nenhum erro.
`relay_pedido` 74 milhões de entradas, `bundle` 273 milhões, `estado_ratchet` 173 milhões,
`envelope` 9,9 milhões, `conteudo` 71 mil (cada uma cifra e decifra numa sessão real).
