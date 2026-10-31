#!/usr/bin/env bash
# Mede do PC (ou de qualquer maquina) o tempo de ida e volta ate cada relay, sem os telemoveis
# pelo meio: tempo da ligacao TCP a porta 443 (= 1 ida e volta). Se aqui for estavel e as apps
# oscilarem, a oscilacao vem das redes dos telemoveis, nao da AWS.
#   bash scripts/latencia-relays.sh            # 60 medidas de 5 em 5 s (~5 min)
#   bash scripts/latencia-relays.sh 20 2       # 20 medidas de 2 em 2 s
N="${1:-60}"; GAP="${2:-5}"
RELAYS="${RELAYS:-relay.sealerapp.com relay-es.sealerapp.com}"
for H in $RELAYS; do : > "/tmp/lat-$H.txt"; done
for i in $(seq 1 "$N"); do
  for H in $RELAYS; do
    curl -s -o /dev/null -w '%{time_connect}\n' --max-time 5 "https://$H/estado.json" >> "/tmp/lat-$H.txt"
  done
  printf '\r  %d/%d' "$i" "$N"; sleep "$GAP"
done
echo
for H in $RELAYS; do
  sort -n "/tmp/lat-$H.txt" | awk -v h="$H" '$1>0{v[n++]=$1*1000} END{
    if(!n){print h ": sem resposta"; exit}
    printf "  %-24s min %4.0f · mediana %4.0f · p95 %4.0f · max %4.0f ms  (%d medidas)\n",
      h, v[0], v[int((n-1)/2)], v[int((n-1)*0.95)], v[n-1], n}'
done
