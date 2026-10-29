# tungstenite 0.24.0 com memoria por ligacao reduzida (Sealer)

Copia do tungstenite 0.24.0 (crates.io, MIT/Apache-2.0), usada no lugar do original
por `[patch.crates-io]` no `Cargo.toml` da raiz. Sem exemplos nem benches.

Porque: o relay tem dezenas de milhares de ligacoes WebSocket quase sempre paradas.
O original guarda em cada ligacao um bloco fixo de 4 KB para ler e a capacidade da
maior mensagem que ja recebeu e enviou (ate ~64 KB depois de um anexo), para sempre.
Medido a 29/10/2026 (8 000 ligacoes, texto e audios): o relay passou de 44 KB por
ligacao parada (88 KB depois de trafego) para 8 KB (12 KB). Ver
`claude/teste_carga_2910.md` (notas do projeto).

Mudancas (procurar "Sealer" no codigo):
- `src/buffer.rs`: le diretamente para o armazenamento, sem o bloco fixo de 4 KB; um
  buffer de leitura vazio e libertado (sem nada a chegar, ou acima de 8 KB).
- `src/protocol/frame/mod.rs`: depois de tudo escrito, o buffer de escrita acima de
  8 KB e libertado.
- `Cargo.toml`: sem os alvos `[[example]]` e `[[bench]]` (ficheiros nao copiados).

O comportamento na rede e o mesmo. Ao subir o tokio-tungstenite para uma versao com
outro tungstenite, rever se isto ainda faz falta (e medir de novo) antes de apagar.
