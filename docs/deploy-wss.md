# Relay atrás de TLS (wss://)

O `relay_server` fala WebSocket + Noise em TCP simples e, por omissão, escuta só
em `127.0.0.1:9443`. O TLS é terminado por um proxy inverso; os clientes ligam
por `wss://`. O Noise (com *pinning* da chave estática do relay) continua por
cima do TLS como segunda camada, independente da PKI.

## Na AWS: HAProxy pelo `deploy/aws/setup-ec2.sh`

O script instala e configura tudo numa EC2 com Ubuntu 24.04: Redis, relay, relay das chamadas
(UDP 443) e o HAProxy com o certificado Let's Encrypt (certbot, renovação automática):

```bash
sudo bash setup-ec2.sh relay.exemplo.org      # sem nome: <ip-com-hifenes>.sslip.io
```

O HAProxy (`/etc/haproxy/haproxy.cfg`, escrito pelo script; não regista nada):

* `:443` com TLS → `127.0.0.1:9443` (relay). `/call` → `127.0.0.1:9445` (relay das chamadas,
  quando o UDP está bloqueado), `/fed` → `127.0.0.1:9446` só para os IPs dos relays de
  `/etc/qt/federacao.json` (os outros recebem 403) e `/estado.json` → `127.0.0.1:9444`.
* `:80` só para o desafio do Let's Encrypt.
* Por IP: no máximo 100 ligações abertas e 100 novas a cada 10 s.
* `timeout tunnel 1h` para as ligações WebSocket abertas.
* **Travões:** `maxconn` = travão do relay + 3000 (no global e no backend do relay) e
  `maxsslrate 300` (handshakes TLS novos por segundo). O travão do relay é
  `RELAY_MAX_CONNECTIONS` em `/etc/qt/relay.env` (25 000 nos relays de produção; cada app aberta
  usa uma ligação). Correr o script de novo mantém os valores; para os mudar:

```bash
sudo RELAY_MAX_CONNECTIONS=25000 bash setup-ec2.sh
sudo HAPROXY_MAXCONN=30000 HAPROXY_MAXSSLRATE=400 bash setup-ec2.sh
```

Com outro proxy (nginx, por exemplo), usar `proxy_http_version 1.1;
proxy_set_header Upgrade $http_upgrade; proxy_set_header Connection "upgrade";` e um
`proxy_read_timeout` ≥ 300 s, igual ao `idle_timeout` do relay.

O relay sozinho:

```bash
REDIS_URL=redis://127.0.0.1:6379 NOISE_KEY_PATH=/var/lib/qt/relay_noise_key.bin ./relay_server
# imprime a chave pública Noise -> usar como "chave do relay" (pinning) na app
```

Na app: URL `wss://relay.exemplo.org` + chave pública de pinning.

## Notas
* O Redis deve escutar só em localhost/rede privada, com um utilizador ACL só
  com os comandos do relay (`deploy/redis/qt-redis.conf`).
* O proxy vê o tráfego já cifrado pelo Noise; o conteúdo continua E2EE.
* `ws://` só é aceite no cliente Rust para loopback (ou com `allow_insecure_ws`
  em testes) e na build *debug* Android.

## Deploy na AWS (EC2): isolamento de rede

Topologia: Internet → `443/tcp` → HAProxy (TLS) → `127.0.0.1:9443` relay → `127.0.0.1:6379` Redis.
Ficam expostas a 443 TCP, a 80 TCP (desafio do Let's Encrypt) e a 443 UDP (relay das chamadas).

1. **Security Group da EC2**: inbound apenas `443/tcp`, `80/tcp` e `443/udp` de `0.0.0.0/0`;
   SSH só a partir do teu IP (ou nenhum, usando SSM Session Manager). **Nunca** abrir `6379`, `9443`,
   `9445` nem `9446`.
2. **Redis**: `deploy/redis/qt-redis.conf` com `bind 127.0.0.1 -::1`, `protected-mode yes`,
   utilizador `default` desligado e utilizador `qt-relay` com ACL mínima (só os comandos e o
   prefixo de chaves do relay). Confirmar com `sudo ss -tlnp | grep 6379`.
3. **Relay**: `deploy/systemd/qt-relay.service` + `/etc/qt/relay.env`. O próprio binário recusa
   arrancar se `RELAY_BIND_ADDR` não for loopback ou se `REDIS_URL` apontar para fora da máquina
   (exceções só com `QT_ALLOW_PUBLIC_BIND=1` / `QT_ALLOW_REMOTE_REDIS=1` + `rediss://`).
4. **Clientes**: o transporte Rust só aceita `wss://` em builds de release (até `ws://localhost`
   é recusado); `ws://` em loopback só existe em builds de debug para os testes locais.
   A app Android release já só aceita `wss://` (network_security_config + verificação no código).

## Imagens e GIFs (anexos)

O relay guarda os anexos, já cifrados pelas apps, em ficheiros no disco e
apaga-os quando o destinatário os descarrega ou ao fim de 24 horas (um varrimento
de 10 em 10 minutos apanha os que ficaram para trás). Configuração em
`/etc/qt/relay.env` (exemplo em `deploy/systemd/qt-relay.env.example`):

| Variável | Omissão | Para quê |
|---|---|---|
| `RELAY_BLOB_DIR` | `anexos/` ao lado da chave Noise (`/var/lib/qt/anexos`) | Pasta dos ficheiros (o serviço só pode escrever em `/var/lib/qt`) |
| `RELAY_BLOB_MAX_TOTAL_MB` | 2048 | Total em disco; acima disto, anexos novos são recusados |
| `RELAY_BLOB_USER_DAILY_MB` | 1024 | Quota diária por utilizador |
| `RELAY_BLOB_MIN_FREE_MB` | 1024 | Espaço que fica sempre livre no disco |
| `RELAY_BLOBS=0` | | Desliga os anexos |

O utilizador `qt-relay` do Redis precisa dos comandos de hash, `DEL` e
`DECRBY` para os anexos; sem eles o envio de imagens falha com "erro interno"
(o teste `server/tests/redis_acl.rs` usa a lista de produção para o apanhar).

## Atualizar o relay

Corre de novo `deploy/aws/setup-ec2.sh`: vai buscar o código ao GitHub,
recompila e reinicia o serviço, mantendo a chave Noise (o pin das apps não
muda) e a password do Redis. A compilação é feita na instância: deixa
espaço em disco (com 8 GB ficou quase cheio).

## ElastiCache

Se mais tarde usares ElastiCache em vez de Redis local: VPC privada, Security Group que só aceita
o SG da EC2 na 6379, *in-transit encryption* ligada e `REDIS_URL=rediss://...` com
`QT_ALLOW_REMOTE_REDIS=1`.
