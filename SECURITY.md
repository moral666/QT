# Política de Segurança

## Reportar uma vulnerabilidade

**Não abra uma issue pública no GitHub para vulnerabilidades de segurança.**
Isso exporia o problema antes de existir uma correção disponível.

Em vez disso:

1. Envie um email para `security@[dominio-do-projeto]` (a definir), cifrado
   com a chave PGP publicada em `docs/security-pgp-key.asc` (a criar).
2. Inclua: descrição do problema, passos para reproduzir, e o impacto
   potencial (ex.: quebra de forward secrecy, bypass de autenticação, etc.).
3. Responderemos em até 72 horas para confirmar a receção.

## Processo de divulgação coordenada (coordinated disclosure)

- Prazo alvo para correção: 90 dias a partir da confirmação, ajustável
  conforme a severidade e complexidade.
- O reportador será creditado publicamente (salvo pedido de anonimato) após
  a correção estar disponível.
- Divulgação pública do problema (CVE, blog post técnico) só ocorre após o
  patch estar disponível para a maioria dos utilizadores.

## Escopo

Áreas de interesse prioritário para reports:
- `core/` — qualquer coisa que comprometa forward secrecy, permita replay,
  ou quebre a autenticação do AEAD
- Implementação do X3DH/PQXDH — bypass de verificação de assinatura,
  downgrade de algoritmos
- Vazamento de metadados não documentado (além do que já é conhecido e
  descrito em `docs/threat-model.md`)

Fora de escopo: engenharia social, ataques que exigem acesso físico
já-privilegiado ao dispositivo, phishing.
