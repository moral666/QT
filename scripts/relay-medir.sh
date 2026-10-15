#!/usr/bin/env bash
# Mede o relay local (release) com o relaybench: CPU e memoria do relay e do Redis,
# comandos do Redis por mensagem, e o que o relaybench escreve (envios/s, latencias).
#
#   ./scripts/relay-medir.sh <nome> -- <argumentos do relaybench>
#   ./scripts/relay-medir.sh 4k -- --pairs 50 --size 4096 --secs 20
#
# Arranca um Redis proprio (porta 6395, sem disco) e o relay (porta 9455) e para
# os dois no fim. Os resultados ficam em medicoes/<nome>.txt.
set -euo pipefail
cd "$(dirname "$0")/.."
NAME="${1:?nome da medicao}"; shift
[ "${1:-}" = "--" ] && shift
OUT="medicoes/$NAME.txt"; mkdir -p medicoes
RPORT=6395; PORT=9455
cargo build -q --release -p qt_server --bin relay_server --example relaybench

redis-server --port $RPORT --save '' --appendonly no --daemonize yes --pidfile /tmp/qt-medir-redis.pid >/dev/null
sleep 0.5
KEY=$(mktemp -u /tmp/qt-medir-key.XXXX)
RELAY_BIND_ADDR=127.0.0.1:$PORT REDIS_URL=redis://127.0.0.1:$RPORT NOISE_KEY_PATH=$KEY \
  RELAY_MAX_CONNECTIONS=20000 RELAY_BLOB_DIR=/tmp/qt-medir-anexos \
  ./target/release/relay_server >/dev/null 2>&1 &
RELAY=$!
trap 'kill $RELAY 2>/dev/null; kill "$(cat /tmp/qt-medir-redis.pid)" 2>/dev/null; rm -f $KEY' EXIT
sleep 0.5
RPID=$(cat /tmp/qt-medir-redis.pid)

ticks() { awk '{print $14+$15}' "/proc/$1/stat"; }
HZ=$(getconf CLK_TCK)
redis-cli -p $RPORT config resetstat >/dev/null

./target/release/examples/relaybench --relay ws://127.0.0.1:$PORT "$@" > /tmp/qt-medir-bench.txt &
BENCH=$!
# CPU medida no meio do teste (o relaybench aquece 3 s e corre --secs).
SECS=$(printf '%s\n' "$@" | awk 'p{print; exit} /^--secs$/{p=1}'); SECS=${SECS:-20}
WIN=$(( SECS > 6 ? SECS - 4 : SECS / 2 )); [ "$WIN" -lt 1 ] && WIN=1
sleep 4
r0=$(ticks $RELAY); d0=$(ticks $RPID); t0=$(date +%s.%N)
sleep "$WIN"
r1=$(ticks $RELAY); d1=$(ticks $RPID); t1=$(date +%s.%N)
RSS_RELAY=$(awk '/VmRSS/ {printf "%.1f", $2/1024}' /proc/$RELAY/status)
RSS_REDIS=$(redis-cli -p $RPORT info memory | awk -F: '/^used_memory_rss:/ {printf "%.1f", $2/1048576}')
wait $BENCH

{
  echo "== $NAME ($(date '+%Y-%m-%d %H:%M'), commit $(git rev-parse --short HEAD))"
  echo "maquina: $(nproc) vCPU, $(awk '/MemTotal/ {printf "%.1f GB", $2/1048576}' /proc/meminfo), $(uname -r)"
  echo "argumentos: $*"
  cat /tmp/qt-medir-bench.txt
  awk -v a="$r0" -v b="$r1" -v c="$d0" -v d="$d1" -v t0="$t0" -v t1="$t1" -v hz="$HZ" \
    'BEGIN {w=t1-t0; printf "CPU (%% de um nucleo, %.0f s a meio): relay %.1f | redis %.1f\n", w, (b-a)/hz/w*100, (d-c)/hz/w*100}'
  echo "memoria no fim: relay ${RSS_RELAY} MB | redis ${RSS_REDIS} MB"
  echo "comandos Redis (todo o teste):"
  redis-cli -p $RPORT info commandstats | awk -F'[:=,]' '/^cmdstat_/ {sub("cmdstat_","",$1); printf "  %s=%s", $1, $3} END {print ""}'
} | tee "$OUT"
