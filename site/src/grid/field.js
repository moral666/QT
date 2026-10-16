/**
 * Grelha infinita de janelas que se arrasta, inspirada no Phantom.land.
 *
 * Feita em DOM e não em WebGL: cada janela é um <article> normal, com texto
 * nítido, selecionável e lido pelos leitores de ecrã. A grelha repete-se em
 * todas as direções: uma janela que sai por um lado volta a entrar pelo outro.
 *
 * Interações:
 *  - arrastar (rato, dedo ou caneta) move a grelha; ao largar, uma mola leva-a
 *    a parar com uma janela ao centro, respeitando o impulso do gesto;
 *  - pressionar faz a grelha recuar um pouco (classe is-pressed, em CSS);
 *  - roda do rato e trackpad movem a grelha;
 *  - setas do teclado avançam uma janela; Tab traz a janela focada ao centro;
 *  - tocar ou clicar numa janela centra-a.
 *
 * O ciclo de desenho só corre enquanto há movimento.
 */
import { FIELD } from './config.js';

export function createField({ field, layer, reducedMotion, onInteract }) {
  const cfg = FIELD;

  let attached = false;
  let sources = []; // janelas originais: { el, marker, row, col }
  let cells = []; // janelas na grelha (originais e cópias): { el, row, col, clone, visible }

  // Medidas
  let vw = 0;
  let vh = 0;
  let cellSize = 0;
  let pitch = 1;
  let cols = cfg.baseCols;
  let rows = cfg.baseRows;
  let patternW = 1;
  let patternH = 1;

  // Estado do movimento (píxeis CSS)
  let ox = 0; // deslocamento da grelha
  let oy = 0;
  let tx = 0; // destino da mola
  let ty = 0;
  let vx = 0; // velocidade (píxeis por segundo)
  let vy = 0;
  let settling = false;
  let dragging = false;
  let drag = null;

  // Desvio discreto a seguir o rato
  let shiftX = 0;
  let shiftY = 0;
  let shiftTargetX = 0;
  let shiftTargetY = 0;

  let frameId = 0;
  let lastTime = 0;
  let wheelTimer = 0;
  let listeners = null;
  let resizeObserver = null;

  const finePointer = matchMedia('(hover: hover) and (pointer: fine)');

  // ---------- Montagem ----------

  /** Leva as janelas (data-cell) para dentro da grelha. */
  function attach() {
    if (attached) return;
    attached = true;

    sources = [...document.querySelectorAll('[data-cell]')].map((el) => {
      const [row, col] = el.dataset.cell.split('-').map(Number);
      const marker = document.createComment('janela');
      el.before(marker);
      return { el, marker, row, col };
    });
    for (const { el } of sources) {
      el.classList.add('is-cell');
      if (!el.hasAttribute('aria-hidden')) el.tabIndex = 0;
      layer.append(el);
    }

    measure();
    // Composição inicial: o centro do ecrã fica entre as quatro janelas
    // centrais (linhas 1 e 2, colunas 2 e 3), atrás do logótipo.
    ox = tx = homeX();
    oy = ty = homeY();
    settling = false;
    vx = vy = 0;
    render();

    listeners = new AbortController();
    const { signal } = listeners;
    field.addEventListener('pointerdown', onPointerDown, { signal });
    field.addEventListener('pointermove', onPointerMove, { signal });
    field.addEventListener('pointerup', onPointerUp, { signal });
    field.addEventListener('pointercancel', onPointerCancel, { signal });
    field.addEventListener('lostpointercapture', onPointerCancel, { signal });
    field.addEventListener('dragstart', (event) => event.preventDefault(), { signal });
    layer.addEventListener('focusin', onFocusIn, { signal });
    window.addEventListener('wheel', onWheel, { passive: false, signal });
    window.addEventListener('keydown', onKeyDown, { signal });
    window.addEventListener('pointermove', onHover, { passive: true, signal });
    document.documentElement.addEventListener('pointerleave', clearHover, { signal });
    // Um elemento focado fora do ecrã não pode fazer a grelha "rolar".
    field.addEventListener('scroll', () => field.scrollTo(0, 0), { signal });

    resizeObserver = new ResizeObserver(() => {
      const oldPitch = pitch;
      measure();
      const ratio = pitch / oldPitch;
      ox *= ratio;
      oy *= ratio;
      tx *= ratio;
      ty *= ratio;
      render();
    });
    resizeObserver.observe(field);
  }

  /** Devolve as janelas aos seus lugares na lista. */
  function detach() {
    if (!attached) return;
    attached = false;
    cancelAnimationFrame(frameId);
    frameId = 0;
    lastTime = 0;
    clearTimeout(wheelTimer);
    listeners?.abort();
    resizeObserver?.disconnect();
    endDrag();
    field.classList.remove('is-moving');
    for (const cell of cells) if (cell.clone) cell.el.remove();
    for (const { el, marker } of sources) {
      el.classList.remove('is-cell');
      el.style.transform = '';
      el.style.visibility = '';
      el.removeAttribute('tabindex');
      marker.replaceWith(el);
    }
    sources = [];
    cells = [];
  }

  /** Calcula o tamanho das janelas e quantas linhas e colunas cobrem o ecrã. */
  function measure() {
    vw = field.clientWidth || innerWidth;
    vh = field.clientHeight || innerHeight;
    const c = cfg.cell;
    const size =
      vw <= c.phoneMaxWidth ? clamp(vw * c.phoneRatio, c.min - 28, c.phoneMax) : clamp(vw * c.widthRatio, c.min, c.max);
    cellSize = Math.round(size);
    pitch = cellSize + Math.round(cellSize * cfg.gapRatio);
    field.style.setProperty('--cell', `${cellSize}px`);
    // Janelas pequenas (telemóveis estreitos): margens e pictogramas mais compactos.
    field.classList.toggle('is-compact', cellSize < 270);

    // A grelha tem de cobrir o ecrã (também quando recua ao pressionar, a 93%)
    // mais meia janela de cada lado, senão ver-se-iam buracos ao dar a volta.
    // Em ecrãs muito grandes, a grelha base repete-se (múltiplos de 6 x 4,
    // para não haver janelas iguais lado a lado).
    const margin = cellSize + 2 * cfg.pointerShift;
    const needCols = Math.ceil((vw / 0.9 + margin) / pitch);
    const needRows = Math.ceil((vh / 0.9 + margin) / pitch);
    const nextCols = cfg.baseCols * Math.max(1, Math.ceil(needCols / cfg.baseCols));
    const nextRows = cfg.baseRows * Math.max(1, Math.ceil(needRows / cfg.baseRows));
    if (nextCols !== cols || nextRows !== rows || cells.length === 0) {
      cols = nextCols;
      rows = nextRows;
      buildCells();
    }
    patternW = cols * pitch;
    patternH = rows * pitch;
  }

  function buildCells() {
    for (const cell of cells) if (cell.clone) cell.el.remove();
    cells = [];
    const byCell = new Map(sources.map((s) => [`${s.row}-${s.col}`, s]));
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const source = byCell.get(`${r % cfg.baseRows}-${c % cfg.baseCols}`);
        if (!source) continue;
        const original = r < cfg.baseRows && c < cfg.baseCols;
        const el = original ? source.el : makeClone(source.el);
        if (!original) layer.append(el);
        cells.push({ el, row: r, col: c, clone: !original, visible: true });
      }
    }
  }

  /** Cópia só visual de uma janela (ecrãs muito grandes): sem ids nem foco. */
  function makeClone(el) {
    const copy = el.cloneNode(true);
    copy.classList.add('is-clone');
    copy.setAttribute('aria-hidden', 'true');
    copy.inert = true;
    copy.removeAttribute('tabindex');
    copy.removeAttribute('aria-labelledby');
    for (const node of [copy, ...copy.querySelectorAll('[id]')]) node.removeAttribute('id');
    return copy;
  }

  // ---------- Desenho ----------

  function requestTick() {
    if (!frameId && attached) {
      // Enquanto a grelha se mexe, cada janela ganha uma camada própria de
      // composição (CSS: .is-moving): mover passa a não exigir redesenhar.
      // Em repouso, as camadas saem e não ocupam memória gráfica.
      field.classList.add('is-moving');
      frameId = requestAnimationFrame(tick);
    }
  }

  function tick(now) {
    frameId = 0;
    // Tempo real (até 0,1 s): mesmo num aparelho lento, a grelha assenta no tempo certo.
    const dt = lastTime ? Math.min((now - lastTime) / 1000, 0.1) : 1 / 60;
    lastTime = now;
    let moving = dragging;

    const follow = 1 - Math.exp(-dt * cfg.pointerEase);
    shiftX += (shiftTargetX - shiftX) * follow;
    shiftY += (shiftTargetY - shiftY) * follow;
    if (Math.abs(shiftTargetX - shiftX) > 0.05 || Math.abs(shiftTargetY - shiftY) > 0.05) moving = true;
    else {
      shiftX = shiftTargetX;
      shiftY = shiftTargetY;
    }

    if (settling && !dragging) {
      // Mola com amortecimento crítico: chega ao destino sem oscilar.
      // Integrada em sub-passos de 1/120 s, para ser estável com qualquer dt.
      const k = cfg.springStiffness;
      const c = 2 * Math.sqrt(k);
      const steps = Math.max(1, Math.ceil(dt * 120));
      const h = dt / steps;
      for (let i = 0; i < steps; i++) {
        vx += (-k * (ox - tx) - c * vx) * h;
        vy += (-k * (oy - ty) - c * vy) * h;
        ox += vx * h;
        oy += vy * h;
      }
      if (Math.abs(ox - tx) < 0.25 && Math.abs(oy - ty) < 0.25 && Math.abs(vx) < 4 && Math.abs(vy) < 4) {
        ox = tx;
        oy = ty;
        vx = vy = 0;
        settling = false;
      } else {
        moving = true;
      }
    }

    render();
    if (moving) {
      frameId = requestAnimationFrame(tick);
    } else {
      lastTime = 0;
      field.classList.remove('is-moving');
    }
  }

  function render() {
    const hw = vw / 2;
    const hh = vh / 2;
    const half = cellSize / 2;
    const { scale, pull } = cfg.lens;
    for (const cell of cells) {
      const x = wrap(cell.col * pitch + ox + shiftX, patternW);
      const y = wrap(cell.row * pitch + oy + shiftY, patternH);
      const nx = x / hw;
      const ny = y / hh;
      const r2 = Math.min(2, (nx * nx + ny * ny) / 2);
      const k = 1 - pull * r2;
      const px = hw + x * k - half;
      const py = hh + y * k - half;
      const visible = px > -cellSize && px < vw && py > -cellSize && py < vh;
      // Janelas fora do ecrã só são atualizadas uma última vez, ao sair.
      if (visible || cell.visible) {
        cell.el.style.transform = `translate3d(${px.toFixed(1)}px, ${py.toFixed(1)}px, 0) scale(${(1 - scale * r2).toFixed(4)})`;
      }
      cell.visible = visible;
    }
  }

  // ---------- Encaixe ----------

  /** Deslocamento que deixa uma janela exatamente ao centro. */
  function snap(value) {
    return Math.round(value / pitch) * pitch;
  }

  function homeX() {
    return -2.5 * pitch;
  }

  function homeY() {
    return -1.5 * pitch;
  }

  function settle() {
    if (reducedMotion()) {
      ox = tx;
      oy = ty;
      vx = vy = 0;
      settling = false;
      render();
      return;
    }
    settling = true;
    requestTick();
  }

  /** Traz uma janela (a cópia mais próxima, se houver várias) para o centro. */
  function centerOn(el) {
    const cell = cells.find((c) => c.el === el);
    if (!cell) return;
    tx = ox - wrap(cell.col * pitch + ox, patternW);
    ty = oy - wrap(cell.row * pitch + oy, patternH);
    settle();
  }

  /** Volta à composição inicial, pelo caminho mais curto. */
  function goHome() {
    if (!attached) return;
    tx = homeX() + Math.round((ox - homeX()) / patternW) * patternW;
    ty = homeY() + Math.round((oy - homeY()) / patternH) * patternH;
    settle();
  }

  /** Janela que fica ao centro quando a grelha chega ao destino (tx, ty). */
  function cellAtTarget() {
    return cells.find(
      (c) => !c.clone && Math.abs(wrap(c.col * pitch + tx, patternW)) < 1 && Math.abs(wrap(c.row * pitch + ty, patternH)) < 1,
    );
  }

  // ---------- Entrada ----------

  function onPointerDown(event) {
    if (drag || (event.pointerType === 'mouse' && event.button !== 0)) return;
    // Um toque num link segue o link (a captura do ponteiro abaixo impediria o clique).
    if (event.target.closest?.('a[href]')) return;
    drag = {
      id: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      lastX: event.clientX,
      lastY: event.clientY,
      lastTime: event.timeStamp,
      startTime: event.timeStamp,
      moved: false,
      tile: event.target.closest('.tile'),
    };
    dragging = true;
    settling = false;
    vx = vy = 0;
    field.setPointerCapture?.(event.pointerId);
    field.classList.add('is-dragging');
    if (!reducedMotion()) field.classList.add('is-pressed');
    onInteract();
    requestTick();
  }

  function onPointerMove(event) {
    if (!drag || event.pointerId !== drag.id) return;
    const dx = event.clientX - drag.lastX;
    const dy = event.clientY - drag.lastY;
    const dt = (event.timeStamp - drag.lastTime) / 1000;
    if (!drag.moved && Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) > cfg.tapMaxMovePx) {
      drag.moved = true;
    }
    ox += dx;
    oy += dy;
    // Eventos muito próximos dariam velocidades absurdas.
    if (dt > 0.004) {
      vx = vx * 0.4 + (dx / dt) * 0.6;
      vy = vy * 0.4 + (dy / dt) * 0.6;
    }
    drag.lastX = event.clientX;
    drag.lastY = event.clientY;
    drag.lastTime = event.timeStamp;
    requestTick();
  }

  function onPointerUp(event) {
    if (!drag || event.pointerId !== drag.id) return;
    const wasTap = !drag.moved && event.timeStamp - drag.startTime < cfg.tapMaxMs;
    const stoppedBeforeRelease = event.timeStamp - drag.lastTime > 80;
    const tile = drag.tile;
    endDrag();
    if (wasTap && tile) {
      centerOn(tile);
      return;
    }
    if (stoppedBeforeRelease) vx = vy = 0;
    const reach = cfg.flickMaxCells * pitch;
    tx = snap(ox + clamp(vx * cfg.flickProjection, -reach, reach));
    ty = snap(oy + clamp(vy * cfg.flickProjection, -reach, reach));
    settle();
  }

  function onPointerCancel(event) {
    if (!drag || event.pointerId !== drag.id) return;
    endDrag();
    tx = snap(ox);
    ty = snap(oy);
    settle();
  }

  function endDrag() {
    if (drag && field.hasPointerCapture?.(drag.id)) field.releasePointerCapture(drag.id);
    drag = null;
    dragging = false;
    field.classList.remove('is-dragging', 'is-pressed');
  }

  function onWheel(event) {
    if (event.ctrlKey) return; // gesto de zoom do trackpad: fica com o navegador
    event.preventDefault();
    const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? vh : 1;
    let dx = event.deltaX * unit;
    let dy = event.deltaY * unit;
    if (event.shiftKey && !dx) {
      dx = dy;
      dy = 0;
    }
    ox -= dx;
    oy -= dy;
    settling = false;
    vx = vy = 0;
    onInteract();
    requestTick();
    clearTimeout(wheelTimer);
    wheelTimer = setTimeout(() => {
      tx = snap(ox);
      ty = snap(oy);
      settle();
    }, cfg.wheelIdleMs);
  }

  function onKeyDown(event) {
    if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return;
    const step = { ArrowLeft: [1, 0], ArrowRight: [-1, 0], ArrowUp: [0, 1], ArrowDown: [0, -1] }[event.key];
    if (!step || event.target.closest?.('input, textarea, select, [contenteditable="true"]')) return;
    event.preventDefault();
    onInteract();
    const baseX = settling ? tx : snap(ox);
    const baseY = settling ? ty : snap(oy);
    tx = baseX + step[0] * pitch;
    ty = baseY + step[1] * pitch;
    settle();
    // Se o foco estava numa janela, passa para a que vai ficar ao centro.
    if (layer.contains(document.activeElement)) cellAtTarget()?.el.focus({ preventScroll: true });
  }

  /** Tab: a janela focada pelo teclado vem para o centro. */
  function onFocusIn(event) {
    const tile = event.target.closest?.('.tile');
    if (!tile || !tile.matches(':focus-visible')) return;
    onInteract();
    centerOn(tile);
  }

  function onHover(event) {
    if (dragging || event.pointerType !== 'mouse' || !finePointer.matches || reducedMotion()) return;
    shiftTargetX = -((event.clientX - vw / 2) / (vw / 2)) * cfg.pointerShift;
    shiftTargetY = -((event.clientY - vh / 2) / (vh / 2)) * cfg.pointerShift;
    requestTick();
  }

  function clearHover() {
    shiftTargetX = 0;
    shiftTargetY = 0;
    requestTick();
  }

  return {
    attach,
    detach,
    goHome,
    /** Estado atual, para testes e para o painel ?debug. */
    get state() {
      return { ox, oy, tx, ty, pitch, cellSize, cols, rows, settling, dragging, cells: cells.length };
    },
  };
}

/** Posição relativa ao centro, dentro de [-tamanho/2, tamanho/2[. */
function wrap(value, size) {
  return ((((value + size / 2) % size) + size) % size) - size / 2;
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}
