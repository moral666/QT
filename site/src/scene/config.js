/**
 * Todos os valores ajustáveis da cena 3D num só sítio.
 *
 * Unidades: 1 unidade = largura da imagem do logótipo (1024 px no ficheiro
 * original). O logótipo ocupa cerca de 0,71 dessa largura.
 */

export const SCENE = {
  /** Tamanho do canvas em relação ao palco. Tem de bater certo com .stage__canvas no CSS (150%). */
  frame: 1.5,
  /** Campo de visão vertical da câmara, em graus. Valores baixos achatam a perspetiva. */
  fov: 26,
  /**
   * Espessura do objeto. Medida na vista de perfil enviada pela SEALER:
   * 38 px de espessura para 272 px de largura (cerca de 14% da largura do
   * balão), ou seja 0,14 x 0,71 = 0,10 da largura da imagem.
   */
  depth: 0.1,
  /** Quanto as paredes laterais incham ao meio (não deve passar o recuo de 2,5 px do contorno). */
  wallBulge: 0.0022,
  /** Subdivisões do perfil da parede, da frente para trás. */
  wallSegments: 10,
  /** Curvatura simulada das normais da parede, em radianos. Dá o aspeto de aresta arredondada. */
  wallNormalBend: 1.05,
};

export const RENDER = {
  /** Limite de densidade de píxeis. Telemóveis com ecrãs 3x não precisam de 3x para este objeto. */
  maxPixelRatioDesktop: 2,
  maxPixelRatioMobile: 1.5,
  minPixelRatio: 1,
  /** Prefere a GPU integrada em portáteis com duas GPUs: gasta menos bateria. */
  powerPreference: 'low-power',
};

export const LOOK = {
  face: {
    roughness: 0.14,
    /** Reflexo do ambiente sobre a imagem. 0 = imagem exata, sem reflexos a mexer. */
    envMapIntensity: 0.55,
  },
  wall: {
    color: '#b8a5ef',
    roughness: 0.22,
    clearcoat: 1,
    clearcoatRoughness: 0.12,
    iridescence: 0.75,
    iridescenceIOR: 1.35,
    iridescenceThicknessRange: [180, 520],
    envMapIntensity: 0.85,
  },
  keyLight: { color: '#e4e9ff', intensity: 1.1, position: [-1.4, 2, 3] },
  rimLight: { color: '#9b7cff', intensity: 1.1, position: [1.6, -0.6, -2.2] },
  /** Poça de luz lilás por baixo do objeto, que encolhe quando ele sobe. */
  pool: { color: '#8a6cff', opacity: 0.32, width: 0.62, height: 0.13, gap: 0.05 },
};

export const MOTION = {
  /** Flutuação vertical. A mesma amplitude da animação CSS do fallback (2,2%). */
  floatAmplitude: 0.022,
  floatPeriod: 7,
  /** Balanço lento quando ninguém está a interagir, em radianos. */
  swayYaw: 0.1,
  swayYawPeriod: 11,
  swayPitch: 0.045,
  swayPitchPeriod: 13,
  /** Inclinação máxima a seguir o rato, em radianos (cerca de 18 e 11 graus). */
  pointerYaw: 0.32,
  pointerPitch: 0.2,
  /** Rapidez com que a inclinação acompanha o rato (maior = mais rápido). */
  pointerEase: 3.2,
  /** Arrastar a largura total do palco roda o objeto 3/4 de volta. */
  dragTurnsPerStage: 0.75,
  /** Quanto o impulso ao largar conta para decidir se dá mais uma volta (segundos). */
  flickProjection: 0.28,
  /** Velocidade máxima de rotação ao arrastar ou lançar (voltas por segundo). */
  maxSpinTurnsPerSecond: 2,
  /** Mola que traz o objeto de volta à frente. Amortecimento crítico: sem oscilar. */
  springStiffness: 16,
  /** Volta completa de apresentação, uma única vez, depois de o 3D aparecer. */
  introDelay: 0.35,
  introDuration: 2.4,
  /** Volta completa ao tocar ou clicar no logótipo. */
  tapSpinDuration: 1.6,
  tapMaxMovePx: 6,
  tapMaxMs: 350,
};

export const QUALITY = {
  /** Janela de medição do tempo médio por fotograma, em segundos. */
  sampleWindow: 2,
  /** Acima disto (cerca de 38 fps), baixa a densidade de píxeis um degrau. */
  slowFrameMs: 26,
  /** Se, já na densidade mínima, continuar acima disto (30 fps) em duas janelas, desiste do 3D. */
  giveUpFrameMs: 34,
  pixelRatioStep: 0.25,
};
