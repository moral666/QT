#!/usr/bin/env bash
# =============================================================================
# Instala o relay QT numa EC2 nova (Ubuntu 24.04, x86_64 ou ARM), de uma vez:
#   Redis (so localhost, ACL)  ->  relay (systemd, so localhost)  ->  HAProxy (TLS, wss://)
#   Relay das chamadas, voz e video (UDP 443 publico; reserva em wss://<nome>/call pelo HAProxy)
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
# Antes: Security Group com entrada so em 22 (o teu IP), 80 e 443 TCP, e 443 UDP (chamadas).
# Nunca 6379/9443/9445.
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
cargo build --release --manifest-path "$SRC/Cargo.toml" -p qt_server --bin relay_server --bin call_relay

# ---------- 4. Utilizador e binario ----------
id qt >/dev/null 2>&1 || useradd --system --home /var/lib/qt --shell /usr/sbin/nologin qt
install -d -o qt -g qt -m 700 /var/lib/qt
# Estado publico (tempo medio de entrega), servido em /estado.json: so este ficheiro, nada mais.
install -d -o qt -g qt -m 755 /var/lib/qt-estado
install -d -o root -g qt -m 750 /etc/qt
install -o root -g root -m 755 "$SRC/target/release/relay_server" /usr/local/bin/qt-relay
install -o root -g root -m 755 "$SRC/target/release/call_relay" /usr/local/bin/qt-call-relay

# ---------- 5. Redis: so localhost + ACL ----------
log "A configurar o Redis (so 127.0.0.1, utilizador com ACL minima)"
if [ -f "$ENV_FILE" ] && grep -q '^REDIS_URL=redis://qt-relay:' "$ENV_FILE"; then
  REDIS_PASS=$(sed -n 's|^REDIS_URL=redis://qt-relay:\([0-9a-f]*\)@.*|\1|p' "$ENV_FILE")
fi
REDIS_PASS="${REDIS_PASS:-$(openssl rand -hex 32)}"
# Fichas do video (0006): o relay das chamadas regista as fichas gastas com um utilizador
# proprio do Redis (qt-fichas: so SET em fichas:*), com a password em /etc/qt-call-relay
# (o qt-call-relay nao le /etc/qt). Mantida entre atualizacoes.
FICHAS_ENV=/etc/qt-call-relay/fichas.env
if [ -f "$FICHAS_ENV" ] && grep -q '^CALL_REDIS_URL=redis://qt-fichas:' "$FICHAS_ENV"; then
  FICHAS_PASS=$(sed -n 's|^CALL_REDIS_URL=redis://qt-fichas:\([0-9a-f]*\)@.*|\1|p' "$FICHAS_ENV")
fi
FICHAS_PASS="${FICHAS_PASS:-$(openssl rand -hex 32)}"
sed -e "s/SUBSTITUI_POR_PASSWORD_FORTE/$REDIS_PASS/" -e "s/SUBSTITUI_POR_PASSWORD_FICHAS/$FICHAS_PASS/" \
  "$SRC/deploy/redis/qt-redis.conf" > /etc/redis/qt-redis.conf
chown redis:redis /etc/redis/qt-redis.conf
chmod 640 /etc/redis/qt-redis.conf
grep -q '^include /etc/redis/qt-redis.conf' /etc/redis/redis.conf || echo 'include /etc/redis/qt-redis.conf' >> /etc/redis/redis.conf
systemctl enable redis-server >/dev/null 2>&1
systemctl restart redis-server

# ---------- 6. Relay (systemd) ----------
log "A configurar o servico do relay"
# Fichas do video por conta e por mes (1 ficha = 1 minuto de video pelo relay): quota configuravel.
# Para mudar: sudo RELAY_FICHAS_POR_MES=600 bash setup-ec2.sh (as atualizacoes seguintes mantem-no).
PREV_FICHAS=""
[ -f "$ENV_FILE" ] && PREV_FICHAS=$(sed -n 's/^RELAY_FICHAS_POR_MES=//p' "$ENV_FILE" | tail -n 1)
case "$PREV_FICHAS" in ''|*[!0-9]*) PREV_FICHAS=300 ;; esac
V_FICHAS="${RELAY_FICHAS_POR_MES:-$PREV_FICHAS}"
case "$V_FICHAS" in ''|*[!0-9]*) echo "Aviso: RELAY_FICHAS_POR_MES='$V_FICHAS' nao e um numero; fica $PREV_FICHAS" >&2; V_FICHAS="$PREV_FICHAS" ;; esac
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
# Fichas do video pelo relay (0006): a semente das chaves de cada mes (criada no primeiro
# arranque; o qt-call-relay so a le) e quantas fichas cada conta recebe por mes.
FICHAS_SEED_PATH=/var/lib/qt/fichas_seed.bin
RELAY_FICHAS_POR_MES=$V_FICHAS
EOF
umask 022
# Notificacoes com a app fechada: Android (FCM) so se existir /etc/qt/fcm.json (a chave da
# conta de servico do Firebase) e iPhone (Apple, sem Firebase) so se existir /etc/qt/apns.json
# (team_id, key_id, topic e a chave .p8, ver server/src/apns.rs). Postos a mao, nunca no git.
# O relay passa entao a poder abrir ligacoes HTTPS para fora (Google e Apple), que o servico
# normalmente proibe (IPAddressDeny).
DROPIN=/etc/systemd/system/qt-relay.service.d
PUSH_OUT=0
if [ -f /etc/qt/fcm.json ]; then
  chown root:qt /etc/qt/fcm.json
  chmod 640 /etc/qt/fcm.json
  echo "QT_FCM_CREDENTIALS=/etc/qt/fcm.json" >> "$ENV_FILE"
  PUSH_OUT=1
  log "Notificacoes Android (FCM): ativas"
else
  log "Notificacoes Android (FCM): desligadas (falta /etc/qt/fcm.json)"
fi
if [ -f /etc/qt/apns.json ]; then
  chown root:qt /etc/qt/apns.json
  chmod 640 /etc/qt/apns.json
  echo "QT_APNS_CREDENTIALS=/etc/qt/apns.json" >> "$ENV_FILE"
  PUSH_OUT=1
  log "Notificacoes iPhone (APNs): ativas"
else
  log "Notificacoes iPhone (APNs): desligadas (falta /etc/qt/apns.json)"
fi
# Denuncias: a chave PUBLICA do operador (hex, 64 caracteres), gerada no computador do operador com
# `cargo run --release -p qt_server --example denuncias -- chave`. Sem ela, as denuncias
# das apps sao guardadas na mesma, mas ninguem as consegue listar nem banir contas.
if [ -f /etc/qt/operator.pub ]; then
  chown root:qt /etc/qt/operator.pub
  chmod 640 /etc/qt/operator.pub
  echo "RELAY_OPERATOR_FILE=/etc/qt/operator.pub" >> "$ENV_FILE"
  log "Denuncias: operador configurado"
else
  log "Denuncias: falta /etc/qt/operator.pub (operador desligado)"
fi
rm -f "$DROPIN/fcm.conf"
if [ "$PUSH_OUT" = 1 ]; then
  mkdir -p "$DROPIN"
  cat > "$DROPIN/notificacoes.conf" <<'EOF2'
# Notificacoes: o relay precisa de falar com fcm.googleapis.com, oauth2.googleapis.com
# e api.push.apple.com (IPs que mudam). Sem isto so falava com localhost.
[Service]
IPAddressDeny=
IPAddressAllow=
EOF2
else
  rm -f "$DROPIN/notificacoes.conf"
fi
chown root:qt "$ENV_FILE"
chmod 640 "$ENV_FILE"
cp "$SRC/deploy/systemd/qt-relay.service" /etc/systemd/system/qt-relay.service
systemctl daemon-reload
systemctl enable qt-relay >/dev/null 2>&1
systemctl restart qt-relay

# Relay das chamadas: processo a parte (UDP publico, sem acesso as contas). Usa a chave Noise
# e a semente das fichas que o qt-relay criou no primeiro arranque; espera por elas.
for _ in $(seq 1 15); do [ -s /var/lib/qt/relay_noise_key.bin ] && [ -s /var/lib/qt/fichas_seed.bin ] && break; sleep 1; done
# Registo das fichas gastas: so o utilizador qt-fichas do Redis (password so aqui, root, 0600;
# o systemd le o ficheiro antes de passar ao utilizador qt).
install -d -o root -g root -m 700 /etc/qt-call-relay
umask 077
printf 'CALL_REDIS_URL=redis://qt-fichas:%s@127.0.0.1:6379\n' "$FICHAS_PASS" > "$FICHAS_ENV"
umask 022
chmod 600 "$FICHAS_ENV"
cp "$SRC/deploy/systemd/qt-call-relay.service" /etc/systemd/system/qt-call-relay.service
# Limites e video das chamadas. NAO vao para o $ENV_FILE: o qt-call-relay nao le /etc/qt (tem
# a password do Redis). Ficam no drop-in limites.conf. Para mudar um valor, passa-o ao
# script UMA vez (ex.: sudo CALL_MONTHLY_EGRESS_GB=120 bash setup-ec2.sh): as atualizacoes
# seguintes mantem o que ja esta no ficheiro. A omissao so vale quando ainda nao ha valor.
#   CALL_MONTHLY_EGRESS_GB   travao do mes: passado isto (GB enviados), video so a 240p (80)
#   CALL_VIDEO_BYTES_PER_SEC video por lado (150000 = ~1,2 Mbit/s, 360p com folga)
#   CALL_RELAY_VIDEO         0 = so voz pelo relay; em ligacao direta o video continua (1)
#   CALL_FICHAS              1 = video pelo relay so com fichas (teto da conta, 0006);
#                            0 = video pelo relay para todos, como antes (1)
#   CALL_MAX_ROOMS (2000), CALL_ROOMS_PER_IP (20), CALL_PACKETS_PER_SEC (100)
# A conta do mes fica em /var/lib/qt-call-relay/mes (StateDirectory do servico): o
# systemd nao a apaga ao atualizar nem ao reiniciar, e este script tambem nao.
CALL_DROPIN=/etc/systemd/system/qt-call-relay.service.d
CALL_CONF="$CALL_DROPIN/limites.conf"
# Valor de NOME: o passado ao script; senao o que ja esta no limites.conf; senao a omissao.
# So numeros (um valor estranho e ignorado, com aviso).
call_val() {
  local name="$1" def="$2" v prev=""
  if [ -f "$CALL_CONF" ]; then
    prev=$(sed -n "s/^Environment=$name=//p" "$CALL_CONF" | tail -n 1)
  fi
  case "$prev" in ''|*[!0-9]*) prev="$def" ;; esac
  v="${!name:-}"
  case "$v" in
    '') v="$prev" ;;
    *[!0-9]*) echo "Aviso: $name='$v' nao e um numero; fica $prev" >&2; v="$prev" ;;
  esac
  printf '%s' "$v"
}
V_MAX_ROOMS=$(call_val CALL_MAX_ROOMS 2000)
V_ROOMS_PER_IP=$(call_val CALL_ROOMS_PER_IP 20)
V_PACKETS_PER_SEC=$(call_val CALL_PACKETS_PER_SEC 100)
V_VIDEO_BPS=$(call_val CALL_VIDEO_BYTES_PER_SEC 150000)
V_EGRESS_GB=$(call_val CALL_MONTHLY_EGRESS_GB 80)
V_RELAY_VIDEO=$(call_val CALL_RELAY_VIDEO 1)
V_CALL_FICHAS=$(call_val CALL_FICHAS 1)
mkdir -p "$CALL_DROPIN"
cat > "$CALL_CONF.novo" <<EOF_CALL
# Gerado pelo setup-ec2.sh. Para mudar, passa o valor ao script uma vez (ex.:
# sudo CALL_MONTHLY_EGRESS_GB=120 bash setup-ec2.sh); as atualizacoes seguintes mantem-no.
[Service]
Environment=CALL_MAX_ROOMS=$V_MAX_ROOMS
Environment=CALL_ROOMS_PER_IP=$V_ROOMS_PER_IP
Environment=CALL_PACKETS_PER_SEC=$V_PACKETS_PER_SEC
Environment=CALL_VIDEO_BYTES_PER_SEC=$V_VIDEO_BPS
Environment=CALL_MONTHLY_EGRESS_GB=$V_EGRESS_GB
Environment=CALL_RELAY_VIDEO=$V_RELAY_VIDEO
Environment=CALL_FICHAS=$V_CALL_FICHAS
Environment=CALL_STATE_FILE=/var/lib/qt-call-relay/mes
EOF_CALL
mv "$CALL_CONF.novo" "$CALL_CONF"
log "Chamadas: travao do video aos ${V_EGRESS_GB} GB/mes, video ${V_VIDEO_BPS} B/s por lado, video pelo relay $([ "$V_RELAY_VIDEO" = 0 ] && echo desligado || echo ligado), $([ "$V_CALL_FICHAS" = 0 ] && echo "sem fichas (para todos)" || echo "com fichas: ${V_FICHAS} por conta e por mes")"
systemctl daemon-reload
systemctl enable qt-call-relay >/dev/null 2>&1
systemctl restart qt-call-relay

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
    # Chamadas de voz quando o UDP esta bloqueado: WebSocket sem conta (qt-call-relay).
    acl chamada path /call
    use_backend chamada if chamada
    default_backend relay

backend relay
    server relay 127.0.0.1:9443 maxconn 40000

backend estado
    http-response set-header Cache-Control no-store
    server estado 127.0.0.1:9444

backend chamada
    # O IP real so serve para os limites por IP (nunca e registado). "set-header"
    # substitui o que o cliente mandar: ninguem consegue fingir outro IP.
    http-request set-header X-Forwarded-For %[src]
    server chamada 127.0.0.1:9445 maxconn 4000
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
systemctl is-active --quiet qt-call-relay && echo "  Chamadas: ativo" || echo "  Chamadas: FALHOU (journalctl -u qt-call-relay); as mensagens funcionam"
if [ "$V_CALL_FICHAS" != 0 ]; then
  if journalctl -u qt-call-relay -n 30 --no-pager -o cat | grep -q 'AVISO'; then
    echo "  Fichas do video: ver os avisos em journalctl -u qt-call-relay (video pelo relay em pausa)"
  else
    echo "  Fichas do video: ativas (${V_FICHAS} por conta e por mes)"
  fi
fi

# Chaves publicas das fichas (0006), para o site (https://sealerapp.com/estado.json, Cloudflare): as
# apps so emitem e gastam fichas com a chave do mes que la estiver, e nao com a que o relay diz.
# So chaves publicas (a semente nunca sai daqui). Vale 24 meses; volta a correr este script antes.
FICHAS_SITE_FILE="${FICHAS_SITE_FILE:-/home/ubuntu/sealer-estado.json}"
[ -d "$(dirname "$FICHAS_SITE_FILE")" ] || FICHAS_SITE_FILE=/root/sealer-estado.json
FICHAS_SITE_URL="${FICHAS_SITE_URL:-https://sealerapp.com/estado.json}"
FICHAS_SITE=""
if FICHAS_SEED_PATH=/var/lib/qt/fichas_seed.bin /usr/local/bin/qt-relay chaves-fichas 24 > "$FICHAS_SITE_FILE.tmp" 2>/dev/null; then
  mv "$FICHAS_SITE_FILE.tmp" "$FICHAS_SITE_FILE"
  chmod 644 "$FICHAS_SITE_FILE"
  [ -d /home/ubuntu ] && [ "$(dirname "$FICHAS_SITE_FILE")" = /home/ubuntu ] && chown ubuntu:ubuntu "$FICHAS_SITE_FILE"
  # O site tem de ter a chave deste mes e a de daqui a 12 meses (senao, publicar ja).
  SITE_JSON=$(curl -s --max-time 10 "$FICHAS_SITE_URL" || true)
  FICHAS_SITE="igual a do site (nada a fazer)"
  for MES in "$(date -u +%Y%m)" "$(date -u -d '+12 months' +%Y%m)"; do
    NOVA=$(grep -o "\"$MES\": *\"[0-9a-f]*\"" "$FICHAS_SITE_FILE" | sed 's/.*"\([0-9a-f]*\)"$/\1/' || true)
    NO_SITE=$(printf '%s' "$SITE_JSON" | grep -o "\"$MES\": *\"[0-9a-f]*\"" | sed 's/.*"\([0-9a-f]*\)"$/\1/' || true)
    if [ -z "$NOVA" ] || [ "$NOVA" != "$NO_SITE" ]; then
      FICHAS_SITE="PUBLICAR: o site nao tem a chave de $MES (sem a do mes o video pelo relay fica em pausa)"
      break
    fi
  done
  echo "  Chaves das fichas para o site: $FICHAS_SITE_FILE ($FICHAS_SITE)"
else
  rm -f "$FICHAS_SITE_FILE.tmp"
  echo "  Chaves das fichas para o site: FALHOU (sem semente? ls -l /var/lib/qt/fichas_seed.bin)"
fi

echo "  Portas abertas para fora (so devem aparecer 22, 80 e 443):"
ss -tlnH | awk '{print $4}' | grep -v -E '^(127\.0\.0\.1|\[::1\]):' | sed 's/^/    /'
echo "  UDP aberto para fora (so deve aparecer 443, chamadas; abrir 443 UDP no Security Group):"
ss -ulnH | awk '{print $4}' | grep -v -E '^(127\.0\.0\.|\[::1\]|\*:|0\.0\.0\.0:68|.*%.*:)' | sed 's/^/    /'

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

 Chaves das fichas do video: ${FICHAS_SITE:-ver acima}
   No teu computador, na pasta do repositorio:
     scp ubuntu@$DOMAIN:$FICHAS_SITE_FILE site/public/estado.json
     git add site/public/estado.json && git commit -m "Chaves das fichas" && git push
   (a Cloudflare publica-o em $FICHAS_SITE_URL)
=====================================================================
EOF
