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
