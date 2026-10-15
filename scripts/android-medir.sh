#!/usr/bin/env bash
# Medicoes da app SEALER num telemovel ligado por USB (depuracao USB ativa).
#
#   ./scripts/android-medir.sh            # um telemovel ligado
#   ANDROID_SERIAL=<serie> ./scripts/android-medir.sh   # varios ligados (ver `adb devices`)
#
# Mede: arranque a frio e a quente, memoria, CPU parado, fluidez ao fazer scroll,
# tamanho do APK e da biblioteca nativa, crashes/ANR; guia os testes manuais de
# rotacao, falta de memoria e rede instavel. Tudo vai para um ficheiro de texto
# que podes partilhar. Nao recolhe mensagens, contactos nem chaves.
set -uo pipefail

PKG="${PKG:-pt.qt.app}"
ACT="${ACT:-pt.qt.app/.MainActivity}"
ADB="${ADB:-$(command -v adb || echo "$HOME/Android/Sdk/platform-tools/adb")}"
[ -x "$ADB" ] || { echo "Nao encontro o adb. Define ADB=/caminho/para/adb"; exit 1; }
"$ADB" get-state >/dev/null 2>&1 || { echo "Nenhum telemovel ligado (adb devices)."; exit 1; }
"$ADB" shell pm path "$PKG" >/dev/null 2>&1 || { echo "A app $PKG nao esta instalada neste telemovel."; exit 1; }

MODEL=$("$ADB" shell getprop ro.product.model | tr -d '\r' | tr ' ' '_')
OUT="sealer-medicoes-${MODEL}-$(date +%Y%m%d-%H%M).txt"
sh() { "$ADB" shell "$@" | tr -d '\r'; }
log() { echo "$*" | tee -a "$OUT"; }
pause() { read -r -p ">>> $1  [Enter para continuar] " _; }

log "== SEALER: medicoes ($(date '+%Y-%m-%d %H:%M')) =="
log "Telemovel: $(sh getprop ro.product.manufacturer) $(sh getprop ro.product.model), Android $(sh getprop ro.build.version.release) (API $(sh getprop ro.build.version.sdk))"
log "RAM total: $(sh cat /proc/meminfo | awk '/MemTotal/ {printf "%.1f GB", $2/1048576}')   CPU: $(sh getprop ro.soc.model) $(sh getprop ro.product.cpu.abi)"
log "Versao da app: $(sh dumpsys package "$PKG" | awk -F= '/versionName/ {print $2; exit}')"

# ---------------------------------------------------------------- tamanho
APK=$(sh pm path "$PKG" | head -1 | sed 's/package://')
log ""
log "== Tamanho =="
log "APK instalado: $(sh stat -c %s "$APK" | awk '{printf "%.1f MB", $1/1048576}')"
TMPAPK=$(mktemp --suffix=.apk)
if "$ADB" pull "$APK" "$TMPAPK" >/dev/null 2>&1 && command -v unzip >/dev/null; then
  unzip -l "$TMPAPK" | awk '/libqt_ffi.so/ {printf "libqt_ffi.so (%s): %.1f MB\n", $4, $1/1048576}' | tee -a "$OUT"
fi
rm -f "$TMPAPK"
log "Dados da app (base cifrada, chaves): $(sh run-as "$PKG" du -sh no_backup 2>/dev/null | awk '{print $1}' || echo 'so em builds debug')"

# --------------------------------------------------------------- arranque
log ""
log "== Arranque (TotalTime em ms, 5 vezes) =="
pause "Se o bloqueio da app estiver ativo, DESATIVA-O agora (as medicoes de arranque contam ate ao 1.o ecra)"
cold=(); warm=()
for _ in 1 2 3 4 5; do
  sh am force-stop "$PKG"; sleep 2
  cold+=("$(sh am start -W -n "$ACT" | awk '/TotalTime/ {print $2}')"); sleep 3
done
for _ in 1 2 3 4 5; do
  sh input keyevent 3; sleep 2
  warm+=("$(sh am start -W -n "$ACT" | awk '/TotalTime/ {print $2}')"); sleep 2
done
log "A frio:   ${cold[*]}"
log "A quente: ${warm[*]}"

# ---------------------------------------------------------------- memoria
mem() { sh dumpsys meminfo "$PKG" | awk '/TOTAL PSS:/ {printf "total %.1f MB", $3/1024} /Native Heap:/ {printf "  nativa %.1f MB", $3/1024} /Java Heap:/ {printf "  java %.1f MB", $3/1024}'; }
log ""
log "== Memoria (PSS) =="
sleep 5
log "Parada no ecra inicial: $(mem)"
pause "Abre 3 conversas diferentes, faz scroll ate ao inicio de uma delas e volta ao ecra inicial"
log "Depois de abrir 3 conversas: $(mem)"

# -------------------------------------------------------------------- CPU
PID=$(sh pidof "$PKG")
cpu() {   # media de N amostras do top (intervalo de 2 s)
  for _ in $(seq 1 "$1"); do sh top -b -d 1 -n 2 -p "$PID" | awk -v p="$PID" '$1==p {v=$9} END {print v}'; sleep 1; done \
    | awk '{s+=$1; n++} END {if (n) printf "%.1f", s/n; else print "?"}'
}
log ""
log "== CPU (% de um nucleo, media) =="
pause "Deixa a app aberta no ecra inicial, sem mexer, durante 1 minuto"
log "App aberta e parada: $(cpu 30) %"
pause "Agora, de OUTRO telemovel, manda mensagens seguidas para este durante 1 minuto (com a conversa aberta aqui)"
log "A receber mensagens: $(cpu 30) %"
sh input keyevent 3
pause "A app esta agora em segundo plano. Espera 1 minuto sem tocar no telemovel"
log "Em segundo plano: $(cpu 30) %"

# --------------------------------------------------------------- fluidez
log ""
log "== Fluidez (scroll numa conversa longa) =="
sh am start -n "$ACT" >/dev/null; sleep 2
sh dumpsys gfxinfo "$PKG" reset >/dev/null
pause "Abre a conversa com MAIS mensagens e faz scroll para cima e para baixo durante 15 segundos; depois carrega Enter"
sh dumpsys gfxinfo "$PKG" | grep -E "Total frames rendered|Janky frames|50th percentile|90th percentile|95th percentile|99th percentile" | tee -a "$OUT"

# ---------------------------------------------------- testes guiados
log ""
log "== Testes guiados (escreve o resultado quando pedido) =="
ask() { read -r -p ">>> $1 Funcionou? (s/n + nota): " r; log "$1 -> $r"; }
sh settings put system accelerometer_rotation 0
for rot in 1 0 1 0; do sh settings put system user_rotation $rot; sleep 2; done
ask "Rotacao: rodei o ecra 4 vezes com uma conversa aberta. A conversa e o texto escrito continuaram la?"
sh input keyevent 3; sleep 1; sh am kill "$PKG"; sleep 2; sh am start -n "$ACT" >/dev/null; sleep 3
ask "Falta de memoria: o sistema matou a app em segundo plano e voltei a abri-la. Abriu bem (e pediu o codigo, se o bloqueio estava ativo)?"
pause "Rede instavel: com a app aberta numa conversa, vou desligar e ligar o Wi-Fi e os dados 3 vezes"
for _ in 1 2 3; do sh svc wifi disable; sh svc data disable; sleep 8; sh svc wifi enable; sh svc data enable; sleep 12; done
ask "Depois de voltar a rede, as mensagens escritas durante a falha foram enviadas e as do outro telemovel chegaram?"

# ------------------------------------------------------- crashes e ANR
log ""
log "== Crashes e ANR registados para a app =="
"$ADB" logcat -b crash -d 2>/dev/null | grep -A20 "$PKG" | head -60 | tee -a "$OUT"
sh dumpsys dropbox --print 2>/dev/null | grep -E "data_app_(crash|anr)" -A3 | grep -B1 -A3 "$PKG" | head -40 | tee -a "$OUT"
log "(vazio acima = nenhum crash nem ANR)"
sh settings put system accelerometer_rotation 1
log ""
log "Concluido. Ficheiro: $OUT"
