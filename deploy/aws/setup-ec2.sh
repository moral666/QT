#!/usr/bin/env bash
# =============================================================================
# Instala o relay QT numa EC2 nova (Ubuntu 24.04, x86_64 ou ARM), de uma vez:
#   Redis (so localhost, ACL)  ->  relay (systemd, so localhost)  ->  HAProxy (TLS, wss://)
#   Certificado: Let's Encrypt pelo certbot (renovacao automatica).
#
# Uso, na EC2:
#   curl -fsSLO https://raw.githubusercontent.com/moral666/QT/security-hardening/deploy/aws/setup-ec2.sh
#   sudo bash setup-ec2.sh                 # nome automatico: <ip-com-hifenes>.sslip.io
#   sudo bash setup-ec2.sh relay.exemplo.pt   # ou um dominio teu ja apontado para o IP
#   sudo bash setup-ec2.sh relay.exemplo.pt 1-2-3-4.sslip.io   # varios nomes ao mesmo tempo
#
# Com varios nomes, o primeiro e o principal (o que as apps novas usam) e todos tem
# certificado. Os nomes ficam em /etc/qt/domains: sem argumentos, o script volta a usa-los.
#
# Pode ser corrido de novo (atualiza o codigo e recompila); a password do Redis
# e a chave Noise do relay sao mantidas.
#
# Antes: Security Group com entrada so em 22 (o teu IP), 80 e 443. Nunca 6379/9443.
# =============================================================================
set -euo pipefail

REPO="https://github.com/moral666/QT.git"
BRANCH="${QT_BRANCH:-security-hardening}"
SRC=/opt/qt-src
ENV_FILE=/etc/qt/relay.env

[ "$(id -u)" -eq 0 ] || { echo "Corre com sudo."; exit 1; }
log() { printf '\n\033[1;34m>>> %s\033[0m\n' "$*"; }

# ---------- 0. Nome publico (para o certificado TLS) ----------
DOMAINS_FILE=/etc/qt/domains
if [ $# -ge 1 ]; then
  DOMAINS="$*"
elif [ -s "$DOMAINS_FILE" ]; then
  DOMAINS=$(cat "$DOMAINS_FILE")
else
  TOKEN=$(curl -fsS -X PUT http://169.254.169.254/latest/api/token -H "X-aws-ec2-metadata-token-ttl-seconds: 60")
  PUBIP=$(curl -fsS -H "X-aws-ec2-metadata-token: $TOKEN" http://169.254.169.254/latest/meta-data/public-ipv4 || true)
  [ -n "$PUBIP" ] || { echo "Sem IP publico: associa um IP elastico a instancia e volta a correr."; exit 1; }
  DOMAINS="${PUBIP//./-}.sslip.io"
fi
DOMAIN="${DOMAINS%% *}"
log "Nomes publicos do relay: $DOMAINS (principal: $DOMAIN)"

# ---------- 1. Pacotes ----------
log "A instalar pacotes (Redis, HAProxy, certbot, ferramentas de compilacao)"
export DEBIAN_FRONTEND=noninteractive
apt-get update -q
apt-get install -y -q build-essential pkg-config git curl ca-certificates redis-server haproxy certbot python3 socat

# ---------- 1b. Servicos que nao fazem falta num relay (memoria e CPU) ----------
# Desligados a mao a 18/10/2026; aqui ficam para cada EC2 nova vir igual.
# O amazon-ssm-agent (Session Manager) fica ligado, a nao ser que QT_DISABLE_SSM=1:
# sem ele so se entra por SSH.
log "A desligar servicos desnecessarios"
for UNIT in multipathd.service multipathd.socket snapd.service snapd.socket snapd.seeded.service \
            fwupd.service fwupd-refresh.timer networkd-dispatcher.service; do
  systemctl disable --now "$UNIT" >/dev/null 2>&1 || true
done
if [ "${QT_DISABLE_SSM:-0}" = 1 ]; then
  systemctl disable --now snap.amazon-ssm-agent.amazon-ssm-agent.service amazon-ssm-agent.service >/dev/null 2>&1 || true
fi

# ---------- 1c. Kernel/TCP para dezenas de milhares de ligacoes paradas ----------
# Cada app tem ligacoes WebSocket quase sempre paradas: buffers pequenos por omissao
# (crescem sozinhos quando ha trafego), mais portas locais (cada ligacao do HAProxy ao
# relay em 127.0.0.1 gasta uma porta; o intervalo normal so tem ~28 000) e filas de
# aceitacao maiores para picos de reconexao.
log "A afinar o kernel (TCP)"
cat > /etc/sysctl.d/90-qt-relay.conf <<'EOF'
net.core.somaxconn = 8192
net.ipv4.tcp_max_syn_backlog = 8192
net.core.netdev_max_backlog = 8192
net.ipv4.ip_local_port_range = 10240 65535
net.ipv4.tcp_rmem = 4096 8192 4194304
net.ipv4.tcp_wmem = 4096 8192 4194304
net.ipv4.tcp_fin_timeout = 15
net.ipv4.tcp_tw_reuse = 1
net.ipv4.tcp_keepalive_time = 300
net.ipv4.tcp_keepalive_intvl = 30
net.ipv4.tcp_keepalive_probes = 5
fs.file-max = 1048576
EOF
sysctl -q --system

# ---------- 2. Swap (instancias de 1-2 GB ficam sem memoria a compilar com LTO) ----------
if ! swapon --show | grep -q /swapfile; then
  log "A criar 2 GB de swap para a compilacao"
  fallocate -l 2G /swapfile && chmod 600 /swapfile && mkswap /swapfile >/dev/null && swapon /swapfile
  grep -q '^/swapfile' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
fi

# ---------- 3. Rust + codigo ----------
if [ ! -x /root/.cargo/bin/cargo ]; then
  log "A instalar Rust (rustup, perfil minimo)"
  curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
# shellcheck disable=SC1091
source /root/.cargo/env

log "A obter o codigo ($BRANCH)"
if [ -d "$SRC/.git" ]; then
  git -C "$SRC" fetch --depth 1 origin "$BRANCH"
  git -C "$SRC" reset --hard FETCH_HEAD
else
  git clone --depth 1 --branch "$BRANCH" "$REPO" "$SRC"
fi

log "A compilar o relay (release; pode demorar 5-15 min numa instancia pequena)"
cargo build --release --manifest-path "$SRC/Cargo.toml" -p qt_server --bin relay_server

# ---------- 4. Utilizador e binario ----------
id qt >/dev/null 2>&1 || useradd --system --home /var/lib/qt --shell /usr/sbin/nologin qt
install -d -o qt -g qt -m 700 /var/lib/qt
# Estado publico (tempo medio de entrega), servido em /estado.json: so este ficheiro, nada mais.
install -d -o qt -g qt -m 755 /var/lib/qt-estado
install -d -o root -g qt -m 750 /etc/qt
install -o root -g root -m 755 "$SRC/target/release/relay_server" /usr/local/bin/qt-relay

# ---------- 5. Redis: so localhost + ACL ----------
log "A configurar o Redis (so 127.0.0.1, utilizador com ACL minima)"
if [ -f "$ENV_FILE" ] && grep -q '^REDIS_URL=redis://qt-relay:' "$ENV_FILE"; then
  REDIS_PASS=$(sed -n 's|^REDIS_URL=redis://qt-relay:\([0-9a-f]*\)@.*|\1|p' "$ENV_FILE")
fi
REDIS_PASS="${REDIS_PASS:-$(openssl rand -hex 32)}"
sed "s/SUBSTITUI_POR_PASSWORD_FORTE/$REDIS_PASS/" "$SRC/deploy/redis/qt-redis.conf" > /etc/redis/qt-redis.conf
chown redis:redis /etc/redis/qt-redis.conf
chmod 640 /etc/redis/qt-redis.conf
grep -q '^include /etc/redis/qt-redis.conf' /etc/redis/redis.conf || echo 'include /etc/redis/qt-redis.conf' >> /etc/redis/redis.conf
systemctl enable redis-server >/dev/null 2>&1
systemctl restart redis-server

# ---------- 6. Relay (systemd) ----------
log "A configurar o servico do relay"
umask 027
cat > "$ENV_FILE" <<EOF
RELAY_BIND_ADDR=127.0.0.1:9443
REDIS_URL=redis://qt-relay:${REDIS_PASS}@127.0.0.1:6379
NOISE_KEY_PATH=/var/lib/qt/relay_noise_key.bin
# Tempo medio de entrega (anonimo), publicado em /estado.json para o site.
RELAY_STATUS_FILE=/var/lib/qt-estado/estado.json
# Imagens/GIFs (ja cifrados pelas apps) em /var/lib/qt/anexos, apagados ao fim de 24 h.
RELAY_BLOB_MAX_TOTAL_MB=${RELAY_BLOB_MAX_TOTAL_MB:-2048}
RELAY_BLOB_USER_DAILY_MB=${RELAY_BLOB_USER_DAILY_MB:-1024}
RELAY_BLOB_MIN_FREE_MB=${RELAY_BLOB_MIN_FREE_MB:-1024}
# Ligacoes em simultaneo (cada app aberta usa 2). Com o HAProxy, ~40 KB por ligacao:
# 8000 sem falhas e ~12 000 a funcionar (medido a 18/10/2026, limite foi o router de
# casa). Travao em 10 000 ate ao teste de 25 000 a partir da AWS.
# Para mudar: sudo RELAY_MAX_CONNECTIONS=20000 bash setup-ec2.sh
RELAY_MAX_CONNECTIONS=${RELAY_MAX_CONNECTIONS:-10000}
EOF
umask 022
# Notificacoes com a app fechada (FCM): so se existir /etc/qt/fcm.json (a chave da conta
# de servico do Firebase, posta a mao; nunca no git). O relay passa entao a poder abrir
# ligacoes HTTPS para fora (Google), que o servico normalmente proibe (IPAddressDeny).
DROPIN=/etc/systemd/system/qt-relay.service.d
if [ -f /etc/qt/fcm.json ]; then
  chown root:qt /etc/qt/fcm.json
  chmod 640 /etc/qt/fcm.json
  echo "QT_FCM_CREDENTIALS=/etc/qt/fcm.json" >> "$ENV_FILE"
  mkdir -p "$DROPIN"
  cat > "$DROPIN/fcm.conf" <<'EOF2'
# Notificacoes (FCM): o relay precisa de falar com fcm.googleapis.com e
# oauth2.googleapis.com (IPs da Google, que mudam). Sem isto so falava com localhost.
[Service]
IPAddressDeny=
IPAddressAllow=
EOF2
  log "Notificacoes (FCM): ativas"
else
  rm -f "$DROPIN/fcm.conf"
  log "Notificacoes (FCM): desligadas (falta /etc/qt/fcm.json)"
fi
chown root:qt "$ENV_FILE"
chmod 640 "$ENV_FILE"
cp "$SRC/deploy/systemd/qt-relay.service" /etc/systemd/system/qt-relay.service
systemctl daemon-reload
systemctl enable qt-relay >/dev/null 2>&1
systemctl restart qt-relay

# ---------- 7. HAProxy (TLS, wss://) + certificado Let's Encrypt ----------
# Antes era o Caddy: medido a 18/10/2026, gastava ~137 KB de RAM por ligacao (o relay
# ~14 KB) e a memoria de um t3.small acabava perto das 8000 ligacoes. O HAProxy so
# ocupa buffers enquanto ha dados a passar. Sem registos de acesso (nem IPs nem horas).
if systemctl list-unit-files caddy.service >/dev/null 2>&1; then
  log "A desligar o Caddy (substituido pelo HAProxy)"
  systemctl disable --now caddy >/dev/null 2>&1 || true
  apt-get purge -y -q caddy >/dev/null 2>&1 || true
fi

# /estado.json: servidor minimo so em localhost, so de leitura, sem registos.
cat > /etc/systemd/system/qt-estado.service <<'UNIT'
[Unit]
Description=QT: /estado.json para o site (so localhost, so leitura)
After=network.target

[Service]
ExecStart=/usr/bin/python3 -m http.server 9444 --bind 127.0.0.1 --directory /var/lib/qt-estado
DynamicUser=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
NoNewPrivileges=yes
ReadOnlyPaths=/var/lib/qt-estado
StandardOutput=null
StandardError=null
Restart=always

[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
systemctl enable qt-estado >/dev/null 2>&1
systemctl restart qt-estado

install -d -m 700 /etc/haproxy/certs

# Parte comum: porta 80 (desafio do Let's Encrypt para o certbot em 127.0.0.1:8888; o
# resto vai para https). Sem "log" em lado nenhum: o HAProxy nao regista nada.
haproxy_base() {
cat <<'CFG'
global
    maxconn 40000
    user haproxy
    group haproxy
    ssl-default-bind-options ssl-min-ver TLSv1.2 no-tls-tickets
    # Buffers de 8 KB em vez de 16 KB: as mensagens WebSocket passam em fluxo (nao
    # precisam de caber no buffer) e os cabecalhos das apps sao pequenos.
    tune.bufsize 8192
    tune.maxrewrite 1024
    # So para consultar contadores (ligacoes recusadas por IP), apenas localmente:
    #   echo "show stat" | sudo socat stdio /var/lib/haproxy/stats.sock
    stats socket /var/lib/haproxy/stats.sock mode 600 level user

defaults
    mode http
    timeout connect 5s
    timeout client 60s
    timeout server 60s
    # WebSocket aberto e parado (a app espera ate 25 s por mensagens).
    timeout tunnel 1h
    # Quem abre a ligacao e nao manda o pedido em 5 s e cortado (antes 10 s).
    timeout http-request 5s
    timeout client-fin 10s

frontend http
    bind :80
    acl acme path_beg /.well-known/acme-challenge/
    use_backend acme if acme
    http-request redirect scheme https code 301 unless acme

backend acme
    server certbot 127.0.0.1:8888
CFG
}

# Um certificado por nome. Na primeira instalacao (ainda sem certificado nenhum) o HAProxy
# arranca so com a porta 80; depois, a configuracao completa ja encaminha o desafio do
# Let's Encrypt, por isso acrescentar um nome nao corta as ligacoes abertas.
for D in $DOMAINS; do
  [ -s "/etc/letsencrypt/live/$D/fullchain.pem" ] && continue
  if ! ls /etc/haproxy/certs/*.pem >/dev/null 2>&1; then
    haproxy_base > /etc/haproxy/haproxy.cfg
    systemctl enable haproxy >/dev/null 2>&1
    systemctl restart haproxy
  fi
  log "A obter o certificado TLS para $D (Let's Encrypt)"
  certbot certonly --standalone --http-01-port 8888 --http-01-address 127.0.0.1 \
    -d "$D" --non-interactive --agree-tos --register-unsafely-without-email --keep-until-expiring \
    || { echo "Falhou o certificado para $D: confirma que o nome aponta para este servidor (DNS) e que a porta 80 esta aberta no Security Group."; exit 1; }
done
echo "$DOMAINS" > "$DOMAINS_FILE"
chmod 644 "$DOMAINS_FILE"

# Depois de cada renovacao (o certbot.timer corre 2x por dia): junta certificado e chave
# no ficheiro que o HAProxy le e recarrega sem cortar as ligacoes abertas.
install -d /etc/letsencrypt/renewal-hooks/deploy
cat > /etc/letsencrypt/renewal-hooks/deploy/haproxy.sh <<'HOOK'
#!/bin/sh
umask 077
D=$(basename "$RENEWED_LINEAGE")
[ -s "/etc/haproxy/certs/$D.pem" ] || exit 0
cat "$RENEWED_LINEAGE/fullchain.pem" "$RENEWED_LINEAGE/privkey.pem" > "/etc/haproxy/certs/$D.pem"
systemctl reload haproxy
HOOK
chmod 755 /etc/letsencrypt/renewal-hooks/deploy/haproxy.sh
# O HAProxy le todos os ficheiros da pasta: so os nomes da lista atual ficam la.
for F in /etc/haproxy/certs/*.pem; do
  [ -e "$F" ] || continue
  case " $DOMAINS " in *" $(basename "$F" .pem) "*) ;; *) rm -f "$F" ;; esac
done
for D in $DOMAINS; do
  ( umask 077; cat "/etc/letsencrypt/live/$D/fullchain.pem" "/etc/letsencrypt/live/$D/privkey.pem" > "/etc/haproxy/certs/$D.pem" )
done

log "A configurar o HAProxy para $DOMAINS"
{
haproxy_base
cat <<CFG

frontend https
    bind :443 ssl crt /etc/haproxy/certs/ alpn http/1.1
    # Limites por IP (decididos a 19/10/2026): no maximo 100 ligacoes abertas ao mesmo
    # tempo e 100 ligacoes novas a cada 10 s. Cada app aberta usa 2 ligacoes, e nas redes
    # moveis muitos clientes saem pelo mesmo IP da operadora (CGNAT). Rever se o contador
    # de recusadas (dcon, ver "stats socket" acima) subir com clientes legitimos.
    stick-table type ipv6 size 100k expire 60s store conn_cur,conn_rate(10s)
    tcp-request connection track-sc0 src
    tcp-request connection reject if { sc0_conn_cur gt 100 } || { sc0_conn_rate gt 100 }
    acl estado path /estado.json
    use_backend estado if estado
    default_backend relay

backend relay
    server relay 127.0.0.1:9443 maxconn 40000

backend estado
    http-response set-header Cache-Control no-store
    server estado 127.0.0.1:9444
CFG
} > /etc/haproxy/haproxy.cfg
haproxy -c -q -f /etc/haproxy/haproxy.cfg || { echo "Configuracao do HAProxy invalida"; exit 1; }
systemctl enable haproxy >/dev/null 2>&1
systemctl restart haproxy

# ---------- 8. Verificacao ----------
log "Verificacao"
sleep 3
systemctl is-active --quiet redis-server && echo "  Redis: ativo"   || { echo "  Redis: FALHOU (journalctl -u redis-server)"; exit 1; }
systemctl is-active --quiet qt-relay     && echo "  Relay: ativo"   || { echo "  Relay: FALHOU (journalctl -u qt-relay)"; exit 1; }
systemctl is-active --quiet haproxy      && echo "  HAProxy: ativo" || { echo "  HAProxy: FALHOU (journalctl -u haproxy)"; exit 1; }
systemctl is-active --quiet qt-estado    && echo "  Estado: ativo"  || echo "  Estado: FALHOU (journalctl -u qt-estado); o resto funciona"

echo "  Portas abertas para fora (so devem aparecer 22, 80 e 443):"
ss -tlnH | awk '{print $4}' | grep -v -E '^(127\.0\.0\.1|\[::1\]):' | sed 's/^/    /'

PIN=""
for _ in $(seq 1 10); do
  PIN=$(journalctl -u qt-relay -n 50 --no-pager -o cat | sed -n 's/.*pinning): \([0-9a-f]\{64\}\).*/\1/p' | tail -1)
  [ -n "$PIN" ] && break
  sleep 1
done

echo "  A verificar o TLS..."
CERT_OK=yes
for D in $DOMAINS; do
  OK=no
  for _ in $(seq 1 30); do
    # Qualquer resposta HTTP (mesmo 400, porque nao e um pedido WebSocket) prova que o TLS funciona.
    if [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "https://$D/" || true)" != "000" ]; then
      OK=yes; break
    fi
    sleep 2
  done
  echo "    $D: $OK"
  [ "$OK" = yes ] || CERT_OK=no
done

cat <<EOF

=====================================================================
 Relay QT instalado.

 Na app (URL do relay):   wss://$DOMAIN
 Chave publica (pinning): ${PIN:-<ver: sudo journalctl -u qt-relay | grep pinning>}

 Certificado TLS: $( [ "$CERT_OK" = yes ] && echo "OK" || echo "ainda nao - confirma que as portas 80 e 443 estao abertas no Security Group e ve: sudo journalctl -u haproxy" )

 Para a build release, em android/gradle.properties:
   qtRelayUrl=wss://$DOMAIN
   qtRelayPin=${PIN:-<chave acima>}
=====================================================================
EOF
