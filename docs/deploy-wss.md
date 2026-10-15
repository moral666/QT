# Relay atrás de TLS (wss://)

O `relay_server` fala WebSocket + Noise em TCP simples e, por omissão, escuta só
em `127.0.0.1:9443`. O TLS é terminado por um proxy inverso; os clientes ligam
por `wss://`. O Noise (com *pinning* da chave estática do relay) continua por
cima do TLS como segunda camada, independente da PKI.

## Exemplo com Caddy (certificado Let's Encrypt automático)

```
relay.exemplo.org {
    reverse_proxy 127.0.0.1:9443
}
```

(O Caddy encaminha WebSockets sem configuração extra. Com nginx, usar
`proxy_http_version 1.1; proxy_set_header Upgrade $http_upgrade; proxy_set_header Connection "upgrade";`
e um `proxy_read_timeout` ≥ 300 s, igual ao `idle_timeout` do relay.)

```bash
REDIS_URL=redis://127.0.0.1:6379 NOISE_KEY_PATH=/var/lib/qt/relay_noise_key.bin ./relay_server
# imprime a chave pública Noise -> usar como "chave do relay" (pinning) na app
```

Na app: URL `wss://relay.exemplo.org` + chave pública de pinning.

## Notas
* O Redis deve escutar só em localhost/rede privada e ter `requirepass`.
* O proxy vê o tráfego já cifrado pelo Noise; o conteúdo continua E2EE.
* `ws://` só é aceite no cliente Rust para loopback (ou com `allow_insecure_ws`
  em testes) e na build *debug* Android.

## Deploy na AWS (EC2): isolamento de rede

Topologia: Internet → `443/tcp` → Caddy/nginx (TLS) → `127.0.0.1:9443` relay → `127.0.0.1:6379` Redis.
Só a porta 443 (e 80, se o Caddy fizer o desafio HTTP-01 do Let's Encrypt) fica exposta.

1. **Security Group da EC2**: inbound apenas `443/tcp` (e `80/tcp` se necessário) de `0.0.0.0/0`;
   SSH só a partir do teu IP (ou nenhum, usando SSM Session Manager). **Nunca** abrir `6379` nem `9443`.
2. **Redis**: `deploy/redis/qt-redis.conf` com `bind 127.0.0.1 -::1`, `protected-mode yes`,
   utilizador `default` desligado e utilizador `qt-relay` com ACL mínima (só os comandos e o
   prefixo de chaves do relay). Confirmar com `sudo ss -tlnp | grep 6379`.
3. **Relay**: `deploy/systemd/qt-relay.service` + `/etc/qt/relay.env`. O próprio binário recusa
   arrancar se `RELAY_BIND_ADDR` não for loopback ou se `REDIS_URL` apontar para fora da máquina
   (exceções só com `QT_ALLOW_PUBLIC_BIND=1` / `QT_ALLOW_REMOTE_REDIS=1` + `rediss://`).
4. **Clientes**: o transporte Rust só aceita `wss://` em builds de release (até `ws://localhost`
   é recusado); `ws://` em loopback só existe em builds de debug para os testes locais.
   A app Android release já só aceita `wss://` (network_security_config + verificação no código).

Se mais tarde usares ElastiCache em vez de Redis local: VPC privada, Security Group que só aceita
o SG da EC2 na 6379, *in-transit encryption* ligada e `REDIS_URL=rediss://...` com
`QT_ALLOW_REMOTE_REDIS=1`.
