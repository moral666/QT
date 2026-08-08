# Política de Segurança

Obrigado por levares a segurança a sério — num projeto como este, isso
importa mais do que quase tudo o resto.

## Reportar uma vulnerabilidade

**Não abras uma issue pública no GitHub para vulnerabilidades de
segurança.** Isso exporia o problema antes de existir uma correção
disponível.

Em vez disso, usa o mecanismo de report privado do próprio GitHub:

1. Vai ao separador **"Security"** deste repositório → **"Report a
   vulnerability"** (ou diretamente em
   `https://github.com/<utilizador>/<repo>/security/advisories/new`).
   Isto cria uma conversa privada, visível só entre ti e o mantenedor,
   sem exigir troca de chaves PGP nem infraestrutura de email dedicada.
2. Inclui: descrição do problema, passos para reproduzir, e o impacto
   potencial (ex.: quebra de forward secrecy, bypass de autenticação, etc.).
3. Vamos responder em até 72 horas para confirmar a receção.

*(Nota: um canal de email com chave PGP pode vir a existir no futuro, se
o projeto crescer para uma equipa maior — por agora, o mecanismo do
GitHub é o caminho real e funcional, em vez de uma promessa por
cumprir.)*

## Processo de divulgação coordenada (coordinated disclosure)

- Prazo alvo para correção: 90 dias a partir da confirmação, ajustável
  conforme a severidade e complexidade.
- O reportador será creditado publicamente (salvo pedido de anonimato)
  depois da correção estar disponível.
- Divulgação pública do problema (CVE, artigo técnico) só acontece depois
  do patch estar disponível para a maioria das pessoas que usam o projeto.

## Onde procurar primeiro

Áreas de maior interesse para quem procura vulnerabilidades:
- `core/` — qualquer coisa que comprometa forward secrecy, permita replay,
  ou quebre a autenticação do AEAD
- A implementação do X3DH/PQXDH — bypass de verificação de assinatura,
  downgrade de algoritmos
- Fugas de metadados não documentadas — ver `docs/protocol-spec.md` para o
  que já é um limite conhecido e assumido (não precisas de reportar isso)

Fora de âmbito: engenharia social, ataques que já exigem acesso físico
privilegiado ao dispositivo, phishing.
