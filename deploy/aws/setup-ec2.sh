#!/usr/bin/env bash
# =============================================================================
# Instala o relay QT numa EC2 nova (Ubuntu 24.04, x86_64 ou ARM), de uma vez:
#   Redis (so localhost, ACL)  ->  relay (systemd, so localhost)  ->  Caddy (TLS, wss://)
#
# Uso, na EC2:
#   curl -fsSLO https://raw.githubusercontent.com/moral666/QT/security-hardening/deploy/aws/setup-ec2.sh
#   sudo bash setup-ec2.sh                 # nome automatico: <ip-com-hifenes>.sslip.io
#   sudo bash setup-ec2.sh relay.exemplo.pt   # ou um dominio teu ja apontado para o IP
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
if [ $# -ge 1 ]; then
  DOMAIN="$1"
else
  TOKEN=$(curl -fsS -X PUT http://169.254.169.254/latest/api/token -H "X-aws-ec2-metadata-token-ttl-seconds: 60")
  PUBIP=$(curl -fsS -H "X-aws-ec2-metadata-token: $TOKEN" http://169.254.169.254/latest/meta-data/public-ipv4 || true)
  [ -n "$PUBIP" ] || { echo "Sem IP publico: associa um IP elastico a instancia e volta a correr."; exit 1; }
  DOMAIN="${PUBIP//./-}.sslip.io"
fi
log "Nome publico do relay: $DOMAIN"

# ---------- 1. Pacotes ----------
log "A instalar pacotes (Redis, Caddy, ferramentas de compilacao)"
export DEBIAN_FRONTEND=noninteractive
apt-get update -q
apt-get install -y -q build-essential pkg-config git curl ca-certificates redis-server caddy

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
# Estado publico (tempo medio de entrega), lido pelo Caddy: so este ficheiro, nada mais.
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
# Tempo medio de entrega (anonimo), publicado pelo Caddy em /estado.json para o site.
RELAY_STATUS_FILE=/var/lib/qt-estado/estado.json
# Imagens/GIFs (ja cifrados pelas apps) em /var/lib/qt/anexos, apagados ao fim de 24 h.
RELAY_BLOB_MAX_TOTAL_MB=${RELAY_BLOB_MAX_TOTAL_MB:-2048}
RELAY_BLOB_USER_DAILY_MB=${RELAY_BLOB_USER_DAILY_MB:-1024}
RELAY_BLOB_MIN_FREE_MB=${RELAY_BLOB_MIN_FREE_MB:-1024}
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

# ---------- 7. Caddy (TLS automatico, wss://) ----------
log "A configurar o Caddy para $DOMAIN"
cat > /etc/caddy/Caddyfile <<EOF
# O Caddy obtem o certificado Let's Encrypt sozinho e encaminha os WebSockets.
$DOMAIN {
    # Estado publico do relay (tempo medio de entrega), lido pelo site sealer.pt.
    handle /estado.json {
        root * /var/lib/qt-estado
        header Cache-Control "no-store"
        file_server
    }
    handle {
        reverse_proxy 127.0.0.1:9443
    }
    log {
        output discard
    }
}
EOF
systemctl enable caddy >/dev/null 2>&1
systemctl restart caddy

# ---------- 8. Verificacao ----------
log "Verificacao"
sleep 3
systemctl is-active --quiet redis-server && echo "  Redis: ativo"   || { echo "  Redis: FALHOU (journalctl -u redis-server)"; exit 1; }
systemctl is-active --quiet qt-relay     && echo "  Relay: ativo"   || { echo "  Relay: FALHOU (journalctl -u qt-relay)"; exit 1; }
systemctl is-active --quiet caddy        && echo "  Caddy: ativo"   || { echo "  Caddy: FALHOU (journalctl -u caddy)"; exit 1; }

echo "  Portas abertas para fora (so devem aparecer 22, 80 e 443):"
ss -tlnH | awk '{print $4}' | grep -v -E '^(127\.0\.0\.1|\[::1\]):' | sed 's/^/    /'

PIN=""
for _ in $(seq 1 10); do
  PIN=$(journalctl -u qt-relay -n 50 --no-pager -o cat | sed -n 's/.*pinning): \([0-9a-f]\{64\}\).*/\1/p' | tail -1)
  [ -n "$PIN" ] && break
  sleep 1
done

echo "  A obter o certificado TLS (pode levar ate 1 min)..."
CERT_OK=no
for _ in $(seq 1 30); do
  # Qualquer resposta HTTP (mesmo 400, porque nao e um pedido WebSocket) prova que o TLS funciona.
  if [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "https://$DOMAIN/" || true)" != "000" ]; then
    CERT_OK=yes; break
  fi
  sleep 2
done

cat <<EOF

=====================================================================
 Relay QT instalado.

 Na app (URL do relay):   wss://$DOMAIN
 Chave publica (pinning): ${PIN:-<ver: sudo journalctl -u qt-relay | grep pinning>}

 Certificado TLS: $( [ "$CERT_OK" = yes ] && echo "OK" || echo "ainda nao - confirma que as portas 80 e 443 estao abertas no Security Group e ve: sudo journalctl -u caddy" )

 Para a build release, em android/gradle.properties:
   qtRelayUrl=wss://$DOMAIN
   qtRelayPin=${PIN:-<chave acima>}
=====================================================================
EOF
