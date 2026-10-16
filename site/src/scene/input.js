/**
 * Liga o rato, o toque e o arrasto ao estado do movimento.
 *
 *  - Rato ou trackpad: o logótipo inclina-se discretamente para o ponteiro.
 *  - Arrastar na horizontal (rato ou dedo): roda o logótipo; ao largar, volta
 *    à frente com inércia. O arrasto vertical continua a fazer scroll
 *    (touch-action: pan-y no CSS do palco).
 *  - Tocar ou clicar sem arrastar: uma volta completa.
 *
 * Devolve uma função que remove todos os listeners.
 */
export function attachInput(stage, motion, cfg) {
  const finePointer = matchMedia('(hover: hover) and (pointer: fine)');
  const controller = new AbortController();
  const { signal } = controller;

  let drag = null;

  function onWindowPointerMove(event) {
    if (!finePointer.matches || event.pointerType !== 'mouse') return;
    const rect = stage.getBoundingClientRect();
    const cx = rect.left + rect.width / 2;
    const cy = rect.top + rect.height / 2;
    motion.setPointer((event.clientX - cx) / (innerWidth / 2), (event.clientY - cy) / (innerHeight / 2));
  }

  function onPointerDown(event) {
    if (event.pointerType === 'mouse' && event.button !== 0) return;
    drag = {
      id: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      lastX: event.clientX,
      lastTime: event.timeStamp,
      startTime: event.timeStamp,
      width: stage.getBoundingClientRect().width || 1,
      moved: false,
    };
    stage.setPointerCapture?.(event.pointerId);
  }

  function onPointerMove(event) {
    if (!drag || event.pointerId !== drag.id) return;
    const dx = event.clientX - drag.startX;
    const dy = event.clientY - drag.startY;
    if (!drag.moved) {
      if (Math.hypot(dx, dy) < cfg.tapMaxMovePx) return;
      // Movimento sobretudo vertical: é scroll, não é rotação.
      if (Math.abs(dy) > Math.abs(dx)) {
        cancel(event);
        return;
      }
      drag.moved = true;
      motion.startDrag();
      stage.classList.add('is-dragging');
    }
    const dt = (event.timeStamp - drag.lastTime) / 1000;
    drag.lastX = event.clientX;
    drag.lastTime = event.timeStamp;
    motion.moveDrag((dx / drag.width) * cfg.dragTurnsPerStage, dt);
  }

  function onPointerUp(event) {
    if (!drag || event.pointerId !== drag.id) return;
    const wasTap = !drag.moved && event.timeStamp - drag.startTime < cfg.tapMaxMs;
    finish(event);
    if (wasTap) motion.spinOnce(cfg.tapSpinDuration);
  }

  function cancel(event) {
    finish(event);
  }

  function finish(event) {
    if (drag?.moved) motion.endDrag();
    stage.classList.remove('is-dragging');
    if (drag && stage.hasPointerCapture?.(drag.id)) stage.releasePointerCapture(drag.id);
    drag = null;
    if (event?.type === 'pointercancel') motion.endDrag();
  }

  window.addEventListener('pointermove', onWindowPointerMove, { passive: true, signal });
  document.documentElement.addEventListener('pointerleave', () => motion.clearPointer(), { signal });
  window.addEventListener('blur', () => motion.clearPointer(), { signal });

  stage.addEventListener('pointerdown', onPointerDown, { signal });
  stage.addEventListener('pointermove', onPointerMove, { signal });
  stage.addEventListener('pointerup', onPointerUp, { signal });
  stage.addEventListener('pointercancel', cancel, { signal });
  stage.addEventListener('dragstart', (event) => event.preventDefault(), { signal });

  return () => {
    controller.abort();
    stage.classList.remove('is-dragging');
  };
}
