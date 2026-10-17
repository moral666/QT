# SEALER, landing page

Página de apresentação da SEALER, a app de mensagens do projeto QT.

O ecrã inteiro é uma grelha de janelas que se arrasta com o rato ou com o dedo, inspirada no site da Phantom. Cada janela tem uma informação: para todos, para especialistas, sobre a arquitetura e sobre testes. Ao centro, por cima da grelha, está o logótipo em 3D, com o nome e a frase principal. Quem preferir ler de seguida tem o botão "Ver em lista".

O núcleo criptográfico do QT não foi tocado: este projeto é independente e vive na pasta `site/` do repositório QT. Está publicado em https://sealer.pt.

## Índice

1. [Instalação e comandos](#1-instalação-e-comandos)
2. [Estrutura do projeto](#2-estrutura-do-projeto)
3. [Como a página funciona](#3-como-a-página-funciona)
4. [Editar as janelas](#4-editar-as-janelas)
5. [Publicar](#5-publicar)
6. [Integrar num site existente](#6-integrar-num-site-existente)
7. [Trocar o logótipo](#7-trocar-o-logótipo)
8. [Ajustar a grelha e o 3D](#8-ajustar-a-grelha-e-o-3d)
9. [Conteúdo a confirmar antes de publicar](#9-conteúdo-a-confirmar-antes-de-publicar)
10. [Desempenho: objetivos e resultados medidos](#10-desempenho-objetivos-e-resultados-medidos)
11. [Compatibilidade](#11-compatibilidade)
12. [Dependências e licenças](#12-dependências-e-licenças)
13. [Decisões técnicas e limitações conhecidas](#13-decisões-técnicas-e-limitações-conhecidas)
14. [Esquema da arquitetura](#14-esquema-da-arquitetura)

## 1. Instalação e comandos

Requisitos: Node.js 20.19 ou mais recente (ou 22.12+), e npm.

```bash
npm install        # instala o Three.js e o Vite
npm run dev        # servidor de desenvolvimento em http://localhost:5173
npm run build      # versão final na pasta dist/
npm run preview    # serve a pasta dist/ em http://localhost:4173
```

A pasta `dist/` é o site pronto a publicar: só ficheiros estáticos, sem servidor de aplicação.

Para ver a página a partir da pasta `dist/` sem instalar nada, num terminal dentro dela: `python3 -m http.server 8000` e abrir http://localhost:8000. Abrir o `index.html` com duplo clique não funciona: os navegadores bloqueiam os scripts de páginas abertas como ficheiro.

Parâmetros úteis no endereço (por exemplo `http://localhost:4173/?debug`):

| Parâmetro | Efeito |
| --- | --- |
| `#lista` | Abre a página no modo lista (texto corrido). |
| `?debug` | Mostra um painel com fps, tempo por fotograma, densidade de píxeis, draw calls e triângulos do logótipo 3D. Serve para medir num telemóvel real. |
| `?static` | Força o logótipo sem 3D (imagem a flutuar em CSS). |
| `?force3d` | Aceita WebGL por software. Só para testes em máquinas sem GPU. |
| `?quality=fixed` | Desliga a qualidade adaptativa do 3D. Só para testes. |

## 2. Estrutura do projeto

```
index.html                  Página: barra, logótipo, grelha e o conteúdo de todas as janelas
politica-de-cookies.html    Política de cookies (só texto, sem JavaScript; estilo em src/legal.css)
privacidade.html            Política de privacidade (exigida pela Google Play; idem)
apagar-conta.html           Como apagar a conta (link web exigido pela Google Play; idem)
termos.html                 Termos de utilização (aceites na app antes de usar; idem)
como-funciona.html          Animação "Como viaja uma mensagem" em 5 passos (src/como-funciona.js e .css)
vite.config.js              Compilação, política de segurança de conteúdo (CSP) e dist/_headers
wrangler.jsonc              Publicação na Cloudflare (Worker sealer, os dois domínios e o endereço do relay)
worker/index.js             Código do Worker: só /api/estado (estado do relay na AWS); o resto é estático
public/
  favicon-32.png, favicon-192.png, apple-touch-icon.png
  og-image.jpg              Pré-visualização para redes sociais (1200 x 630)
src/
  main.js                   Arranque: modos da página e, se compensar, o logótipo 3D
  layout.js                 Modos: grelha (início e explorar) e lista
  status.js                 Marcador de estado do serviço (ao lado do nome, na barra de topo)
  capabilities.js           Regras para carregar ou não o 3D
  styles.css                Todo o estilo da página
  legal.css                 Estilo das páginas de texto (política de cookies, como funciona)
  como-funciona.js, .css    Animação da página "Como funciona" (textos dos passos em SCENES)
  grid/
    config.js               Valores ajustáveis da grelha
    field.js                Grelha infinita: arrasto, encaixe, lente, roda, teclado, toque
  scene/
    config.js               Valores ajustáveis do logótipo 3D
    logoScene.js            Cena Three.js (montagem, ciclo de desenho, pausa, limpeza)
    geometry.js             Paredes laterais extrudidas a partir do contorno
    motion.js               Flutuação, inclinação, volta, arrasto com inércia
    input.js                Rato, toque e arrasto do logótipo
    quality.js              Qualidade adaptativa
    debugHud.js             Painel ?debug
  assets/logo/              Logótipo recortado (96, 512, 768, 1024 px) e contorno
  assets/fonts/             Unbounded e Manrope (só caracteres latinos) e licenças
  assets/noise.png          Textura mínima que evita bandas de cor no brilho
scripts/prepare_logo.py     Recorta um logótipo novo e gera todos os ficheiros dele
scripts/architecture_diagram.py
                            Gera o esquema da arquitetura do QT (../docs/, em português e inglês)
tests/run_tests.py          Testes automáticos de compatibilidade e desempenho
tests/results/              Resultados e capturas da última execução (criada pelos testes, fora do git)
```

## 3. Como a página funciona

A página é feita para estar completa mesmo nas piores condições, e cada camada extra só entra quando compensa.

**Ao abrir**

1. Um script mínimo, dentro do próprio HTML, escolhe o modo antes de a página aparecer: grelha (por omissão) ou lista (se o endereço terminar em `#lista`). Sem JavaScript, a página mostra a lista.
2. O logótipo, o nome e a frase aparecem primeiro. O logótipo é o conteúdo principal e é o terceiro ficheiro a ser pedido.
3. Depois de o logótipo estar desenhado, a grelha monta-se e aparece com um desvanecimento.
4. Depois de a página carregar, `capabilities.js` decide se carrega o logótipo 3D. Não carrega com "reduzir movimento" ligado, com poupança de dados, em aparelhos com menos de 2 GB de memória, ou quando o navegador só tem WebGL por software. Quando carrega, o primeiro fotograma 3D coincide com a imagem, por isso a troca não se nota, e o logótipo dá uma volta completa, uma única vez.

**Na grelha**

- Arrastar (rato, dedo ou caneta) move a grelha em qualquer direção. A grelha é infinita: uma janela que sai por um lado volta a entrar pelo outro.
- Ao largar, a grelha pára sempre com uma janela ao centro, com uma mola que respeita o impulso do gesto (um lançamento avança no máximo duas janelas).
- Ao pressionar, a grelha recua um pouco. Longe do centro, as janelas encolhem e escurecem, como numa lente grande-angular. Com rato, a grelha desvia-se ligeiramente no sentido contrário ao ponteiro.
- Tocar ou clicar numa janela traz essa janela ao centro. A roda do rato e o trackpad movem a grelha. As setas avançam uma janela. Tab percorre as janelas e traz cada uma ao centro.
- Ao primeiro gesto, o logótipo e a frase saem de cena e o nome passa para o canto de cima. O botão com o nome, ou a tecla Esc, volta ao início.
- O logótipo 3D roda quando se arrasta em cima dele, e não é desenhado enquanto se explora a grelha.
- O ciclo de desenho da grelha só corre enquanto há movimento. Enquanto se mexe, cada janela ganha uma camada própria de composição (não é redesenhada a cada fotograma); em repouso, essas camadas saem, para não ocupar memória.

**No modo lista**

As mesmas janelas aparecem em texto corrido, agrupadas por público, com scroll normal. É também a versão para leitores de ecrã e para navegadores sem JavaScript.

**Se algo falhar**

Se o aparelho não aguentar o 3D, a página baixa a resolução e, se não chegar, volta à imagem com um cruzamento suave. Se a imagem do logótipo falhar, aparece um balão desenhado em CSS. Se o Three.js falhar, fica a imagem. Se a grelha falhar, a página passa para o modo lista.

## 4. Editar as janelas

Todo o conteúdo das janelas está no `index.html`, dentro de `<main class="list">`, agrupado como aparece no modo lista. Cada janela é um `<article>`:

```html
<article class="tile tile--todos" data-cell="1-2" aria-labelledby="t-so-tu">
  <p class="tile__label tile__label--group">Para todos</p>
  <svg class="tile__icon" viewBox="0 0 48 48" aria-hidden="true">...</svg>
  <h3 class="tile__title" id="t-so-tu">Só tu e quem recebe</h3>
  <p class="tile__text">Ninguém pelo caminho consegue ler as tuas mensagens. Nem a&nbsp;SEALER.</p>
</article>
```

- `data-cell="linha-coluna"` diz onde fica a janela na grelha de 6 colunas por 4 linhas (linhas 0 a 3, colunas 0 a 5). Cada posição só pode ter uma janela. As quatro janelas à volta do centro no início são as das linhas 1 e 2, colunas 2 e 3.
- A classe de categoria dá a cor: `tile--todos` e `tile--passo` (lilás), `tile--tecnico` e `tile--arquitetura` (azul frio), `tile--testes tile--figure` (número em destaque), `tile--proximo` (contorno tracejado, para o que ainda não está feito).
- `tile__label--group` esconde o rótulo no modo lista, onde o título da secção já diz o mesmo.
- O `id` do título tem de ser único e igual ao `aria-labelledby` da janela.
- Mantenha o texto curto: cerca de 120 caracteres no máximo. Os testes verificam, em 8 tamanhos de ecrã, se o texto cabe em todas as janelas.
- Os pictogramas são SVG com traço (`viewBox="0 0 48 48"`), sem cor própria: a cor vem do CSS.

Depois de editar: `npm run build` e `python3 tests/run_tests.py`.

## 5. Publicar

O site está na Cloudflare, num Worker chamado `sealer` que só tem ficheiros estáticos, com os domínios `sealer.pt` e `www.sealer.pt`. O domínio está registado na dominios.pt e usa os nameservers da Cloudflare. A configuração está em `wrangler.jsonc`.

**Publicar uma versão nova**, dentro desta pasta:

```bash
npm run build
npx wrangler login     # só da primeira vez: abre o navegador para entrar na conta da Cloudflare
npx wrangler deploy    # envia a pasta dist/ para o Worker sealer
```

A primeira versão foi carregada no painel da Cloudflare, por arrastar e largar. As seguintes publicam-se daqui, com o wrangler. Na primeira vez, o wrangler pode avisar que o Worker foi criado no painel: confirme.

Em `wrangler.jsonc`, `workers_dev: true` mantém também o endereço `sealer.nunojsilvag.workers.dev`. Para o site existir só em sealer.pt, mude para `false` e publique outra vez.

**Publicar sozinho a cada push (opcional).** No painel da Cloudflare: Workers & Pages → `sealer` → Settings → Builds, ligar ao repositório QT, com a pasta raiz `site`, o comando de compilação `npm run build` e o comando de publicação `npx wrangler deploy`.

**Noutro alojamento.** Envie o conteúdo de `dist/` para qualquer alojamento de sites estáticos.

A política de segurança de conteúdo (CSP) vai embutida no HTML, e a compilação gera também `dist/_headers` com a mesma política, que a Cloudflare (Workers e Pages) e a Netlify aplicam. O ficheiro de cabeçalhos acrescenta `frame-ancestors`, que não funciona dentro do HTML.

A CSP autoriza o único script embutido pela sua impressão digital (SHA-256), calculada em cada compilação. Por isso, noutros servidores, copie a linha `Content-Security-Policy` de `dist/_headers` depois de cada compilação, em vez de a escrever à mão:

**nginx**

```nginx
# valor copiado de dist/_headers
add_header Content-Security-Policy "default-src 'none'; script-src 'self' 'sha256-...'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'" always;
add_header Referrer-Policy "no-referrer" always;
add_header X-Content-Type-Options "nosniff" always;
location /assets/ { add_header Cache-Control "public, max-age=31536000, immutable"; }
```

**Apache (.htaccess)**

```apache
# valor copiado de dist/_headers
Header always set Content-Security-Policy "default-src 'none'; script-src 'self' 'sha256-...'; ..."
Header always set Referrer-Policy "no-referrer"
Header always set X-Content-Type-Options "nosniff"
```

Ative a compressão gzip ou brotli no servidor: os pesos indicados na secção 10 assumem compressão.

A imagem de pré-visualização para redes sociais (`og:image`, no `index.html`) aponta para `https://sealer.pt/og-image.jpg`. Se o domínio mudar, atualize esse endereço: as redes sociais só aceitam endereços completos.

## 6. Integrar num site existente

**Numa subpasta.** Os caminhos são relativos, por isso `dist/` funciona tal como está em `https://site.exemplo/sealer/`. Se o site principal enviar a sua própria CSP, ela tem de permitir scripts, estilos, imagens e tipos de letra do próprio domínio (`'self'`) e a impressão digital do script embutido (ver `dist/_headers`).

**Só o logótipo 3D, ou só a grelha, dentro de outra página.** O código de cada um está isolado: `src/scene/` (logótipo) e `src/grid/` (grelha). A grelha precisa de um elemento `.field` com uma camada `.field__layer`, e de elementos com `data-cell` na página; o logótipo precisa do palco (`<div class="stage">` e o seu conteúdo). Copie a marcação e os blocos de CSS correspondentes, e compile com `npm run build`.

Em Shopify, os ficheiros de `dist/assets/` vão para a pasta `assets` do tema e a marcação para uma secção própria, sem alterar o cabeçalho nem os outros componentes do tema. Este caminho não foi testado numa loja Shopify real.

## 7. Trocar o logótipo

1. Prepare uma imagem quadrada (1024 x 1024 recomendado), com fundo branco liso ou transparente.
2. Corra o script:

   ```bash
   pip install pillow numpy scipy scikit-image
   python3 scripts/prepare_logo.py caminho/para/novo-logo.png
   ```

   O script recorta o fundo, gera as versões em WebP (incluindo o ícone da barra), o contorno usado nas paredes 3D e os ícones do separador.
3. Se o novo logótipo tiver outra espessura, ajuste `SCENE.depth` em `src/scene/config.js` (a explicação do cálculo está no próprio ficheiro).
4. `npm run build`, e confirme com `python3 tests/run_tests.py`. O ficheiro `tests/results/angulos-0-30-60-90-120-180.png` mostra o objeto nos seis ângulos.
5. Atualize `public/og-image.jpg` se quiser a pré-visualização nova nas redes sociais.

## 8. Ajustar a grelha e o 3D

**Grelha** (`src/grid/config.js`):

| Valor | O que muda |
| --- | --- |
| `cell.widthRatio`, `min`, `max` | Tamanho das janelas em ecrãs largos. |
| `cell.phoneRatio`, `phoneMax` | Tamanho das janelas em telemóveis. |
| `gapRatio` | Espaço entre janelas. |
| `lens.scale`, `lens.pull` | Força do efeito de lente (0 = sem efeito). |
| `pointerShift` | Quanto a grelha se desvia a seguir o rato (0 = sem desvio). |
| `springStiffness` | Rapidez do encaixe ao largar. |
| `flickProjection`, `flickMaxCells` | Quanto um lançamento avança. |

O recuo ao pressionar está em `styles.css` (`.field.is-pressed .field__zoom`), e o escurecimento por trás do logótipo em `.js.mode-grid .hero::before`.

**Logótipo 3D** (`src/scene/config.js`):

| Valor | O que muda |
| --- | --- |
| `SCENE.depth` | Espessura do objeto. |
| `LOOK.wall.color`, `iridescence` | Cor e reflexo arco-íris das paredes laterais. |
| `LOOK.face.envMapIntensity` | Força dos reflexos sobre o logótipo (0 = imagem exata). |
| `MOTION.floatAmplitude`, `floatPeriod` | Amplitude e ritmo da flutuação. |
| `MOTION.introDuration` | Duração da volta de apresentação. |
| `RENDER.maxPixelRatioMobile` | Resolução máxima em telemóveis (1,5 por omissão). |
| `QUALITY.slowFrameMs`, `giveUpFrameMs` | Quando baixar a resolução e quando desistir do 3D. |

## 9. Conteúdo a confirmar antes de publicar

As janelas descrevem o QT tal como foi descrito durante o projeto. Confirme estes pontos na versão que vai ser publicada:

- **Proteção pós-quântica** (janelas "Pensada para o futuro" e "PQXDH"): só é verdade se a app publicada for compilada com a funcionalidade `pq` ativa. Se não for, retire as duas.
- **Remetente selado** (janelas "Viaja sempre fechada" e "Remetente selado"): depende de o sealed sender estar ativo na versão publicada.
- **iPhone** (janelas "Também trancadas no telemóvel", "Núcleo em Rust" e "Ligação ao relay"): confirmado em `ios/README.md`. A app de iPhone usa o mesmo núcleo em Rust, o histórico cifrado (com a chave protegida pelo Secure Enclave) e a chave do relay fixada. A janela "Armazenamento" fala só do Android Keystore.
- **Servidor na União Europeia**: o relay está em Estocolmo (eu-north-1). Se mudar de região, atualize a janela.
- **O que vem a seguir**: lista funcionalidades planeadas para o lançamento. Retire as que deixarem de estar previstas, e mova para outras janelas as que já estiverem feitas.
- **Números de "Testes e números"**: "5 aparelhos" vem do teste entre 2 Android, 1 emulador, 1 iPhone e 1 Mac. "0,8 s" é o tempo medido até esta página aparecer num telemóvel lento (secção 10). Se a página mudar muito, volte a medir.
- **Rodapé e "0 cookies"**: verdade para esta página. Se mais tarde acrescentar analítica, atualize as duas.

## 10. Desempenho: objetivos e resultados medidos

Medições feitas com `tests/run_tests.py` no Chromium 141 (Linux, sem GPU). Os números de carregamento, da grelha e do caminho sem 3D são fiáveis. Sem GPU, o WebGL é feito por software, por isso **os números do logótipo 3D são pessimistas e não representam um telemóvel real**.

| Objetivo | Resultado medido | Estado |
| --- | --- | --- |
| HTML, CSS e JS iniciais abaixo de 15 KB (comprimidos) | 14,9 KB (inclui o texto de todas as janelas e a grelha) | Cumprido |
| Primeira vista no telemóvel abaixo de 200 KB | 110,3 KB | Cumprido |
| Com 3D, abaixo de 400 KB | 245,4 KB (o módulo 3D são 135 KB, pedidos depois de a página carregar) | Cumprido |
| Logótipo visível (LCP) abaixo de 2,5 s num telemóvel com CPU 4x mais lento e rede 4G lenta | 0,77 s (com e sem 3D); 0,29 s em desktop | Cumprido |
| Primeiro desenho (FCP) no mesmo telemóvel | 0,60 a 0,64 s | Medido |
| Arrastar a grelha sem recálculos de layout por fotograma | 5 recálculos no total de cada arrasto (só nas mudanças de modo) | Cumprido |
| 60 fps ao arrastar a grelha | Fotograma mediano de 16,7 ms em desktop e no telemóvel com CPU 4x mais lento | Cumprido |
| A grelha em repouso não gasta processador | 0,1 a 0,3 ms por segundo | Cumprido |
| O logótipo 3D não é desenhado quando não está à vista | 0 fotogramas a explorar a grelha, fora do ecrã na lista e com o separador escondido | Cumprido |
| A troca imagem para 3D não se nota | Silhuetas sobrepostas a 99,97% (IoU 0,9997); diferença média de cor 4 em 255 | Cumprido |
| Draw calls e triângulos do 3D baixos | 4 draw calls, 6 046 triângulos, 3 shaders | Cumprido |
| Aparelho lento volta à imagem sem falhar | Densidade 1,5, depois 1,25, depois 1, depois cruzamento para a imagem | Cumprido |
| Bloqueio do navegador no caminho sem 3D (telemóvel, CPU 4x mais lento) | A maior tarefa tem 271 ms: é a montagem da grelha, depois de o logótipo aparecer | Medido |
| **50 fps ou mais com o 3D num telemóvel de gama média** | **Não medido** (não há GPU no ambiente de testes) | Por medir |
| **Bloqueio ao preparar o 3D num aparelho real** | **Não medido.** Com WebGL por software chegou a 3,4 s em desktop e 1,4 s no teste de telemóvel. Nos navegadores com a extensão KHR_parallel_shader_compile, a página compila os shaders em paralelo antes do primeiro fotograma. | Por medir |

Como medir o que falta: abra a página publicada com `?debug` num telemóvel de gama média (Android com Chrome e iPhone com Safari), espere 10 segundos e anote os fps e os ms por fotograma do painel. Se os fps do 3D ficarem abaixo de 38, a página baixa a resolução sozinha; abaixo de 30 na resolução mínima, volta à imagem.

## 11. Compatibilidade

- **Testado (16 testes automáticos):** Chromium 141 em desktop e em emulação de telemóvel (390 x 844, ecrã 3x, toque). Casos: com 3D, sem GPU, sem WebGL, movimento reduzido, versão estática, logótipo em falta, Three.js em falta; arrasto, encaixe, toque, setas, roda, Esc e lista na grelha; 8 tamanhos de ecrã de 320 a 2560 px sem deslocamento horizontal e com o texto a caber; Tab pelas janelas, lista pelo teclado, ids únicos; consola sem erros nem avisos; CSP sem violações.
- **Não testado:** Safari (macOS e iPhone), Firefox e telemóveis Android reais. Recomendo testar nestes três antes do lançamento, sobretudo o arrasto com o dedo no iPhone.
- **Requisitos da página:** navegadores atuais (Chrome e Edge 108+, Safari 16+, Firefox 112+), por causa das unidades `svh`, `overflow: clip` e do atributo `inert`.
- **Requisitos do 3D:** WebGL 2 com GPU (Safari 15+, Chrome 56+, Firefox 51+). Sem isso, o logótipo fica como imagem estática; a grelha funciona na mesma.

## 12. Dependências e licenças

| Componente | Versão | Licença | Uso |
| --- | --- | --- | --- |
| Three.js | 0.186.1 | MIT | Logótipo 3D, no site |
| Vite | 8.3.4 | MIT | Compilação, não vai para o site |
| Unbounded (tipo de letra variável, latino) | fontsource 5 | SIL Open Font License 1.1 | Marca e títulos; licença em `src/assets/fonts/` |
| Manrope (tipo de letra variável, latino) | fontsource 5 | SIL Open Font License 1.1 | Texto; licença em `src/assets/fonts/` |
| Pillow, NumPy, SciPy, scikit-image | atuais | HPND/MIT-CMU, BSD-3 | Só no script `prepare_logo.py` |
| fontTools, Brotli, uharfbuzz | atuais | MIT, MIT, Apache 2.0 | Só no script `architecture_diagram.py` |
| Playwright | 1.56 | Apache 2.0 | Só nos testes |
| Wrangler | 4 (por `npx`) | MIT ou Apache 2.0 | Só para publicar na Cloudflare |
| Logótipo e pictogramas da SEALER | | Propriedade da SEALER | Imagem, objeto 3D e pictogramas desenhados para este site |

A grelha não usa bibliotecas. Nenhum serviço externo é usado pela página: sem CDNs, sem tipos de letra de terceiros, sem analítica, sem cookies.

O marcador de estado (ao lado do nome, na barra de topo) pergunta ao próprio site em `/api/estado`. Quem responde é o Worker (`worker/index.js`): tenta abrir uma ligação WebSocket ao relay da AWS (endereço em `RELAY_URL`, no `wrangler.jsonc`), fecha-a logo e guarda o resultado 30 segundos. Verde: o relay aceitou a ligação. Vermelho: não aceitou (servidor desligado, Caddy ou relay parados). Cinzento: o site não respondeu, por isso o estado é desconhecido. O navegador do visitante nunca fala com a AWS. Se o relay mudar de endereço, basta alterar `RELAY_URL`.

O marcador mostra também o tempo médio de envio na última hora ("envio 23 ms"): quanto tempo uma mensagem demora a ir do telemóvel de quem a envia até ao servidor. Quem o mede é o próprio relay (`server/src/latency.rs`): logo depois de aceitar uma mensagem, manda um ping a esse telemóvel e mede a ida e volta até ao pong; o envio é metade. Só em memória, sem remetente, destinatário nem tamanho. O relay mede também o tempo de entrega (desde que recebe a mensagem até o destinatário confirmar que a tem, sem contar mensagens que esperam mais de 60 segundos), publicado no mesmo ficheiro mas não mostrado no marcador. O relay escreve o resultado a cada minuto em `RELAY_STATUS_FILE`, o Caddy publica-o em `https://<relay>/estado.json`, e o Worker junta-o à resposta de `/api/estado`. Sem mensagens na última hora (ou depois de reiniciar o relay), o marcador mostra a última medição conhecida: o relay guarda-a no próprio `estado.json` (`ultima_ida_ms`, `ultima_ida_medicao`; para a entrega, `ultima_media_ms`, `ultima_medicao`) e lê-a de novo ao arrancar. A frase completa (ao passar o rato) diz há quanto tempo foi medida.

## 13. Decisões técnicas e limitações conhecidas

- **Grelha em DOM, e não em WebGL como no site da Phantom.** As janelas são texto: em DOM, a letra fica nítida em qualquer ecrã, pode ser selecionada, traduzida pelo navegador e lida pelos leitores de ecrã. Os efeitos (lente, recuo, desvio a seguir o rato, inércia) são calculados por janela, com transformações que não obrigam a recalcular o layout. A distorção é uma aproximação por janela: cada janela encolhe e aproxima-se do centro, mas as suas arestas continuam retas.
- **Cópias de janelas em ecrãs grandes.** Para a grelha nunca mostrar buracos, em ecrãs muito altos ou muito largos a grelha base de 6 x 4 repete-se (48 a 96 janelas, das quais 24 são as verdadeiras). As cópias são só visuais: sem foco e escondidas dos leitores de ecrã.
- **A grelha monta-se depois do logótipo.** O logótipo é o conteúdo principal e não espera pela grelha. Num telemóvel lento, a grelha aparece cerca de meio segundo depois.
- **Three.js em vez de `<model-viewer>`.** O objeto 3D é construído a partir da imagem e do contorno do logótipo, sem ficheiro GLB. É preciso controlar o primeiro fotograma (para coincidir com a imagem), a volta de apresentação, o arrasto com mola, a pausa e a qualidade adaptativa.
- **As faces do 3D são a imagem.** A frente e o verso mostram a imagem original; a espessura vem das paredes extrudidas. O "S" e os três pontos não têm relevo próprio, e a luz desenhada na imagem não muda com a rotação.
- **WebGL por software é recusado de propósito.** Em computadores sem GPU, o 3D seria lento e gastaria bateria.
- **Modo de poupança de energia do iPhone.** Limita as animações a 30 fps; a página pode baixar a resolução do 3D nesse caso.
- **CSP.** A diretiva `frame-ancestors` só funciona em cabeçalhos do servidor (ver secção 5). O servidor de desenvolvimento (`npm run dev`) corre sem CSP.
- **Tipos de letra.** Só incluem caracteres latinos (português, inglês, espanhol, francês...). Para outras escritas é preciso acrescentar outros subconjuntos.
- **Números de desempenho do 3D.** Foram medidos com WebGL por software e são pessimistas. Faltam medições num telemóvel real (ver secção 10).

## 14. Esquema da arquitetura

Os READMEs do repositório mostram um esquema da arquitetura de todo o QT: `docs/arquitetura.svg` (em português) e `docs/architecture.svg` (em inglês), na raiz do repositório. Os dois são gerados por `scripts/architecture_diagram.py`, com as cores e os tipos de letra deste site.

Para mudar um texto ou acrescentar um componente, edite `CONTENT` no início do script (nas duas línguas) e volte a gerar, dentro desta pasta:

```bash
pip install fonttools brotli uharfbuzz
python3 scripts/architecture_diagram.py
```

O texto é convertido em contornos, por isso o esquema fica igual em qualquer navegador e no GitHub, sem depender das fontes de quem o vê. Se um texto novo não couber na sua caixa, o script pára e diz qual é.

Quando a arquitetura mudar (por exemplo, notificações push ou federação de relays), atualize o esquema junto com a documentação.
