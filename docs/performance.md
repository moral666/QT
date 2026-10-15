# Desempenho do relay

Medições de quanto o relay gasta por ligação. Cada entrada diz o que foi
medido, como repetir a medição e o que ficou de fora.

---

## 15 de outubro de 2026: imagens e GIFs

Tempo de cada passo de um anexo, num PC, com o relay na mesma máquina (sem rede
real: num telemóvel, enviar e descarregar dependem sobretudo da rede; cifrar,
verificar e ler demoram 3 a 5 vezes mais):

| Tamanho | Cifrar | Enviar ao relay | Descarregar | Verificar | Ler para o ecrã |
|---|---|---|---|---|---|
| 300 KB (foto comprimida) | 3 ms | 7 ms | 8 ms | 4 ms | 0,5 ms |
| 5 MB | 28 ms | 63 ms | 86 ms | 18 ms | 10 ms |
| 25 MB (máximo) | 114 ms | 0,30 s | 0,43 s | 83 ms | 51 ms |

As apps enviam 8 blocos de 40 KB seguidos antes de esperar pelas respostas.
Com o algoritmo de Nagle ativo no relay, cada resposta pequena ficava retida à
espera do ACK do cliente e enviar 25 MB demorava 3,6 s; o relay passou a
desligá-lo (`TCP_NODELAY`) em todas as ligações. Teste: `server/tests/attachments_e2e.rs`
(o mesmo caminho, em `--release`, com um ficheiro de 25 MiB).

---

## 15 de outubro de 2026: a app deixa de abrandar com o histórico

Três consultas à base de dados percorriam o histórico inteiro sempre que eram
chamadas, por isso demoravam mais a cada mensagem guardada. Foram acrescentados
dois índices (`messages_by_msg_id` e o parcial `messages_unread`) e a lista de
conversas passou a ler a última mensagem de cada contacto pelo índice. As bases
existentes ganham os índices sozinhas ao abrir.

Tempo de cada operação no núcleo (chamada pelos bindings, num PC; num telemóvel
médio conta com 3 a 5 vezes mais), com poucas mensagens por ler:

| Operação | 0 mensagens | 12 000 mensagens, antes | 12 000 mensagens, depois |
|---|---|---|---|
| Receber uma mensagem e confirmar o envio | 4,8 ms | 17,5 ms | **5,1 ms** |
| Marcar a conversa como lida | 0,5 ms | 24,9 ms | **0,6 ms** |
| Lista de conversas | 0,1 ms | 16,7 ms | **2,5 ms** |
| Enviar uma mensagem | 2,1 ms | 3,2 ms | 3,1 ms |
| Página de 50 mensagens | 0,2 ms | 0,5 ms | 0,7 ms |

O teste `storage/tests/query_plans.rs` confirma que estas consultas usam os índices.

---

## 15 de outubro de 2026: ligação ao Redis partilhada

Até aqui, o relay abria uma ligação TCP nova ao Redis em cada pedido. Passou a
usar uma só ligação partilhada (`redis::aio::ConnectionManager`), que volta a
ligar sozinha se o Redis reiniciar. Mesmo teste da entrada seguinte (10 000
ligações a pedir mensagens de 3 em 3 s, mesma máquina):

| | Antes | Depois |
|---|---|---|
| Ligações abertas / falhas | 10 000 / 0 | 10 000 / 0 |
| Pedidos respondidos | ~3 300 por segundo | ~3 300 por segundo |
| Memória do relay | 139 MB | 138 MB |
| Memória do Redis | 11 MB | 7 MB |
| CPU do relay | 33 % de um núcleo (~0,10 ms por pedido) | **17 %** (~0,05 ms por pedido) |
| CPU do Redis | 21 % (~0,06 ms por pedido) | **7 %** (~0,02 ms por pedido) |
| **CPU total** | **54 %** de um núcleo | **24 %** (−56 %) |

Reinício do Redis com o relay ligado (200 ligações a pedir mensagens de 0,5 em
0,5 s, Redis desligado durante 5 s): os pedidos ficaram em espera durante a
falha, houve 1 erro interno e, assim que o Redis voltou, tudo continuou sem
erros e sem reiniciar o relay.

---

## 15 de outubro de 2026: 10 000 ligações em simultâneo

### Resultado

| | 10 000 ligações a pedir mensagens de 3 em 3 s (como as apps abertas) | 10 000 ligações paradas |
|---|---|---|
| Ligações abertas ao mesmo tempo | **10 000 de 10 000, 0 falhas** | **10 000 de 10 000, 0 falhas** |
| Memória do relay (RSS) | **139 MB** no total, **~14 KB por ligação** | 137 MB, ~13 KB por ligação |
| Memória do Redis | 11 MB | 7 MB |
| Pedidos respondidos | ~3 300 por segundo, **0 erros** (298 154 no total) | - |
| CPU do relay | 33 % de um núcleo, cerca de **0,10 ms por pedido** | 0 % |
| CPU do Redis | 21 % de um núcleo, cerca de 0,06 ms por pedido | 0 % |

O relay arranca com 4 MB de memória. Cada ligação acrescenta cerca de 14 KB.
Uma ligação parada não gasta CPU: o custo de CPU vem dos pedidos que as apps
fazem enquanto estão abertas.

### Como foi medido

- Relay (`relay_server`, compilado em `--release`) e Redis na mesma máquina:
  2 vCPU Intel Xeon @ 2,10 GHz, 8 GB de RAM, Linux.
- Cliente: `server/examples/conntest.rs`. Cada ligação faz o mesmo caminho que
  as apps: WebSocket → handshake Noise XX → autenticação por assinatura → um
  pedido de mensagens a cada 3 s (o intervalo das apps), cifrado com Noise.
- Memória: `VmRSS` de `/proc/<pid>/status` com todas as ligações abertas.
- CPU: tempo de CPU do processo (`utime + stime` de `/proc/<pid>/stat`) a meio
  do teste, durante 45 s.

Para repetir:

```bash
redis-server --port 6390 --save '' --appendonly no --daemonize yes
ulimit -n 20000
RELAY_BIND_ADDR=127.0.0.1:9450 REDIS_URL=redis://127.0.0.1:6390 \
  NOISE_KEY_PATH=/tmp/relay_noise_key.bin RELAY_MAX_CONNECTIONS=20000 \
  cargo run --release -p qt_server --bin relay_server &

# noutro terminal (tambem com ulimit -n 20000):
cargo run --release -p qt_server --example conntest -- \
  --relay ws://127.0.0.1:9450 --conns 10000 --poll-ms 3000 --secs 90
# com --poll-ms 0 as ligacoes ficam abertas mas paradas
```

### O que ficou de fora (e muda os números reais)

- **TLS:** em produção, o Caddy trata do TLS (`wss://`) antes do relay e gasta
  memória e CPU próprios por ligação, que não entram nesta medição.
- **Rede real:** cliente e relay correram na mesma máquina. Com telemóveis em
  redes móveis, o sistema operativo guarda mais dados em memória por ligação.
- **Máquina:** o CPU usado não é o da instância da AWS. Os valores de CPU são
  uma indicação, não uma garantia para outra máquina.
- **Uma só identidade:** todas as ligações usam a mesma identidade. Não houve
  envio de mensagens nem de recibos, só os pedidos de mensagens das apps abertas
  (o caso mais frequente). Para envios e latência de ponta a ponta ver
  `server/examples/loadtest.rs`.

### Configuração necessária para tantas ligações

Por omissão, o relay aceita até 4 096 ligações (`RELAY_MAX_CONNECTIONS`). O
serviço `deploy/systemd/qt-relay.service` já sobe o limite de ficheiros abertos
para 65 536 (`LimitNOFILE`); sem isso, o systemd limitava o relay a cerca de
1 024 ligações. Para mais de 4 096 ligações em simultâneo, sobe
`RELAY_MAX_CONNECTIONS` em `/etc/qt/relay.env`.
