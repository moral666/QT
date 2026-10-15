# Desempenho do relay

Medições de quanto o relay gasta por ligação. Cada entrada diz o que foi
medido, como repetir a medição e o que ficou de fora.

---

## 15 de outubro de 2026 — 10 000 ligações em simultâneo

### Resultado

| | 10 000 ligações a pedir mensagens de 3 em 3 s (como as apps abertas) | 10 000 ligações paradas |
|---|---|---|
| Ligações abertas ao mesmo tempo | **10 000 de 10 000, 0 falhas** | **10 000 de 10 000, 0 falhas** |
| Memória do relay (RSS) | **139 MB** no total, **~14 KB por ligação** | 137 MB, ~13 KB por ligação |
| Memória do Redis | 11 MB | 7 MB |
| Pedidos respondidos | ~3 300 por segundo, **0 erros** (298 154 no total) | — |
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

Por omissão, o relay aceita até 4 096 ligações (`RELAY_MAX_CONNECTIONS`) e o
systemd limita cada serviço a cerca de 1 024 ficheiros abertos. Para mais
ligações em simultâneo é preciso subir os dois: `RELAY_MAX_CONNECTIONS` em
`/etc/qt/relay.env` e `LimitNOFILE` no serviço `qt-relay`.
