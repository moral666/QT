# Rascunho para a política de privacidade (NÃO publicar)

Este ficheiro não entra no site (o site só usa a pasta `site/`). Guarda o texto que saiu da política de privacidade pública no 0008, para quando o Nuno tratar da parte legal. Antes de publicar algum destes parágrafos, rever com quem tratar da parte legal e traduzir para as dez línguas.

## Fichas anónimas (vídeo pelo servidor, desde o 0006)

Para a lista "O que passa pelo nosso servidor":

> **Fichas anónimas**: contamos quantas fichas anónimas foram emitidas para a tua conta em cada mês, para evitar abusos do servidor. O servidor das chamadas guarda, de cada ficha usada, só um resumo que não diz de que conta é, para a mesma ficha não ser usada duas vezes.

Para a lista "Durante quanto tempo" (valores atuais do código: `FICHAS_ISSUED_TTL_SECONDS` em `server/src/store.rs` e o prazo do `SET` em `server/src/fichas.rs`):

> Fichas anónimas: o número de fichas emitidas para a tua conta num mês, 40 dias; o resumo de cada ficha usada, até ao dia seguinte ao fim do mês dela. Na app, as fichas ficam na base de dados cifrada e são apagadas quando são usadas e quando o mês acaba.

Para a lista "Quem mais está envolvido" (Cloudflare), se o parágrafo acima for publicado:

> A app consulta também um ficheiro público no nosso site (sealer.pt, alojado na Cloudflare) para confirmar que as fichas são iguais para toda a gente. Esse pedido é igual para todos e não leva o teu ID.

## Acesso às funções pagas (tirado da política pública no 0008)

Estes dois itens estavam na política publicada (chaves `priv.18d` e `priv.26d`) e saíram por causa da regra de não falar de funções pagas no site. O relay continua a guardar estes dados (`{p}:premium:{conta}`, ver `docs/protocol-spec.md`, secção 23), por isso a política tem de voltar a dizê-lo antes de o Premium chegar a alguém fora dos testes.

> **Acesso às funções pagas** (só se o tiveres): até que data a tua conta tem esse acesso, ligada ao teu ID, para o servidor saber se pode aceitar as tuas chamadas e anexos.

> Acesso às funções pagas: até 30 dias depois de terminar, ou até apagares a conta.
