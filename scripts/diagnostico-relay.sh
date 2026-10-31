#!/usr/bin/env bash
# Diagnostico do relay, SO LEITURA (nao muda nada). Corre no proprio relay:
#   sudo bash /opt/qt-src/scripts/diagnostico-relay.sh
# Sem IPs de clientes nem ids de contas: so contagens e configuracao.
set -uo pipefail
[ "$(id -u)" -eq 0 ] || { echo "Corre com sudo."; exit 1; }
t() { printf '\n== %s\n' "$*"; }

t "Maquina"
echo "  $(hostname) · $(nproc) vCPU · arranque: $(uptime -s) · carga: $(cut -d' ' -f1-3 /proc/loadavg)"
echo "  CPU roubada pelo hipervisor (st, 5 s; deve ser 0):"
vmstat 1 5 | awk 'NR==2{for(i=1;i<=NF;i++) if($i=="st") c=i} NR>2{s+=$c; n++} END{printf "    media st = %.1f%%\n", s/n}'

t "Memoria e swap"
free -m | sed 's/^/  /'
for P in qt-relay redis-server haproxy qt-call-relay; do
  PID=$(pgrep -xo "$P") || { echo "  $P: nao esta a correr"; continue; }
  awk -v p="$P" '/^VmRSS/{r=$2} /^VmSwap/{s=$2} END{printf "  %-14s RAM %6d MB · na swap %5d MB\n", p, r/1024, s/1024}' "/proc/$PID/status"
done

t "Redis ↔ relay (ligacao parada fechada pelo Redis)"
echo "  timeout no qt-redis.conf: $(sed -n 's/^timeout //p' /etc/redis/qt-redis.conf) (deve ser 0)"
echo "  'falha na limpeza' dos anexos nos ultimos 7 dias: $(journalctl -u qt-relay --since '7 days ago' -o cat --no-pager | grep -c 'falha na limpeza')"

t "Kernel (TCP)"
for K in vm.swappiness net.ipv4.tcp_slow_start_after_idle net.ipv4.tcp_congestion_control net.core.default_qdisc; do
  printf '  %-36s %s\n' "$K" "$(sysctl -n $K 2>/dev/null)"
done
IF=$(ip -o route get 1.1.1.1 | sed -n 's/.* dev \([^ ]*\).*/\1/p')
echo "  qdisc em $IF: $(tc qdisc show dev "$IF" | head -1 | awk '{print $2}')"

t "TLS"
for C in /etc/letsencrypt/live/*/cert.pem; do
  [ -e "$C" ] || continue
  printf '  %-28s %s · expira %s\n' "$(basename "$(dirname "$C")")" \
    "$(openssl x509 -in "$C" -noout -text | sed -n 's/.*Public Key Algorithm: //p' | head -1)" \
    "$(openssl x509 -in "$C" -noout -enddate | cut -d= -f2)"
done
echo "  cache de sessoes no HAProxy: $(grep -E 'tune.ssl.(cachesize|lifetime)' /etc/haproxy/haproxy.cfg | awk '{printf "%s=%s ", $1, $2}')"
D=$(head -1 /etc/qt/domains | awk '{print $1}')
rm -f /tmp/qt-sess.bin
echo | timeout 5 openssl s_client -connect 127.0.0.1:443 -servername "$D" -sess_out /tmp/qt-sess.bin >/dev/null 2>&1
R=$(echo | timeout 5 openssl s_client -connect 127.0.0.1:443 -servername "$D" -sess_in /tmp/qt-sess.bin 2>/dev/null | grep -oE '^(Reused|New)')
rm -f /tmp/qt-sess.bin
echo "  retoma de sessao: ${R:-falhou o teste} (deve ser Reused)"

t "HAProxy"
echo "show info" | socat stdio /var/lib/haproxy/stats.sock 2>/dev/null | grep -E '^(CurrConns|MaxConn|SslRate|SslRateLimit|CurrSslConns|SslCacheLookups|SslCacheMisses):' | sed 's/^/  /'
echo "show stat" | socat stdio /var/lib/haproxy/stats.sock 2>/dev/null | awk -F, 'NR==1{for(i=1;i<=NF;i++) h[$i]=i; next} $1=="https" && $2=="FRONTEND"{print "  recusadas por IP (dcon): " $h["dcon"]}'

t "Relay (ultima linha de estatisticas)"
journalctl -u qt-relay -n 200 -o cat --no-pager | grep -E 'ativas|ida_' | tail -1 | fold -w 110 | sed 's/^/  /'
echo "  reinicios do relay em 7 dias: $(journalctl -u qt-relay --since '7 days ago' -o cat --no-pager | grep -c 'pinning')"

t "Ligacao aos outros relays (TCP, 5 tentativas)"
if [ -f /etc/qt/federacao.json ]; then
  python3 -c 'import json;[print(p["nome"], p["url"].split("/")[2]) for p in json.load(open("/etc/qt/federacao.json"))["pares"]]' |
  while read -r N H; do
    V=$(for _ in 1 2 3 4 5; do curl -s -o /dev/null -w '%{time_connect}\n' --max-time 3 "https://$H/estado.json"; done |
        awk '{v=$1*1000; s+=v; if(n==0||v<mn)mn=v; if(v>mx)mx=v; n++} END{printf "min %.0f · media %.0f · max %.0f ms", mn, s/n, mx}')
    echo "  $N ($H): $V"
  done
else
  echo "  sem federacao"
fi

t "Discos"
df -h / /var/lib/redis /var/lib/qt 2>/dev/null | awk 'NR==1 || !s[$1]++' | sed 's/^/  /'
