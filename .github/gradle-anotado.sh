#!/usr/bin/env bash
# Corre o Gradle e, se falhar, publica as linhas de erro como anotacoes do GitHub
# (visiveis na pagina da execucao e pela API, sem descarregar o registo inteiro).
set -o pipefail
gradle --no-daemon --console=plain "$@" 2>&1 | tee /tmp/gradle.log
code=$?
if [ $code -ne 0 ]; then
  {
    grep -E '^e: |^w: .*(error)|ERROR:|Missing class|What went wrong' -A6 /tmp/gradle.log
    grep -E -A25 'What went wrong' /tmp/gradle.log
  } | head -120 | sed -e 's/%/%25/g' | awk 'NF' | while IFS= read -r line; do
    echo "::error::${line:0:900}"
  done
  # Lint: resumo dos erros do relatorio de texto, se existir.
  for f in app/build/reports/lint-results-*.txt; do
    [ -f "$f" ] && grep -E 'Error:' "$f" | head -40 | while IFS= read -r line; do echo "::error::${line:0:900}"; done
  done
fi
exit $code
