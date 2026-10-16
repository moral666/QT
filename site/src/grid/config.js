/**
 * Valores ajustáveis da grelha de janelas (modo grelha).
 * Medidas em píxeis CSS, tempos em segundos, salvo indicação.
 */
export const FIELD = {
  /** Grelha base: cada janela tem data-cell="linha-coluna" dentro destes limites. */
  baseCols: 6,
  baseRows: 4,

  /** Tamanho de cada janela quadrada. */
  cell: {
    /** Ecrãs largos: fração da largura do ecrã, entre min e max. */
    widthRatio: 0.23,
    min: 268,
    max: 460,
    /** Telemóveis (até phoneMaxWidth): fração maior, para se ler bem. */
    phoneMaxWidth: 640,
    phoneRatio: 0.74,
    phoneMax: 320,
  },
  /** Espaço entre janelas, em fração do tamanho da janela. */
  gapRatio: 0.075,

  /**
   * Efeito de lente (como uma objetiva grande-angular): longe do centro, as
   * janelas encolhem e aproximam-se do centro. r² vai de 0 (centro) a 1 (canto).
   */
  lens: { scale: 0.16, pull: 0.07 },

  /** Desvio discreto da grelha no sentido contrário ao rato (só com rato). */
  pointerShift: 26,
  pointerEase: 5,

  /** Mola do encaixe: depois de largar, a grelha pára com uma janela ao centro. */
  springStiffness: 70,
  /** Quanto o impulso do arrasto conta para escolher a janela onde parar. */
  flickProjection: 0.22,
  /** Um lançamento avança no máximo este número de janelas. */
  flickMaxCells: 2,

  /** Toque ou clique sem arrastar: centra a janela tocada. */
  tapMaxMovePx: 6,
  tapMaxMs: 350,

  /** Depois da roda do rato parar durante este tempo (ms), a grelha encaixa. */
  wheelIdleMs: 140,
};
