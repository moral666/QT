/**
 * Estado do movimento do logótipo, sem dependências do Three.js.
 *
 * Camadas, combinadas em update():
 *  - flutuação vertical e balanço lento (sempre ativos);
 *  - inclinação a seguir o rato (só com rato ou trackpad);
 *  - rotação em Y ("spin"): volta de apresentação, toque, arrasto com
 *    inércia e uma mola que traz sempre a frente de volta.
 *
 * No instante zero todas as camadas valem zero, para o primeiro fotograma
 * 3D coincidir com a imagem estática que estava no ecrã.
 */

const TAU = Math.PI * 2;

export function createMotion(cfg) {
  let time = 0;

  const pointer = { x: 0, y: 0 };
  const pointerTarget = { x: 0, y: 0 };

  let spin = 0;
  let spinVelocity = 0;
  let restAngle = 0; // múltiplo de 2π para onde a mola puxa
  let tween = null;
  let dragging = false;
  let dragStartSpin = 0;

  function update(dt) {
    time += dt;

    const follow = 1 - Math.exp(-dt * cfg.pointerEase);
    pointer.x += (pointerTarget.x - pointer.x) * follow;
    pointer.y += (pointerTarget.y - pointer.y) * follow;

    if (tween) {
      tween.elapsed += dt;
      const p = Math.min(1, tween.elapsed / tween.duration);
      spin = tween.from + (tween.to - tween.from) * easeInOutCubic(p);
      if (p >= 1) {
        tween = null;
        spinVelocity = 0;
      }
    } else if (!dragging) {
      // Mola com amortecimento crítico: chega à frente sem ultrapassar.
      const k = cfg.springStiffness;
      const c = 2 * Math.sqrt(k);
      const acceleration = -k * (spin - restAngle) - c * spinVelocity;
      spinVelocity += acceleration * dt;
      spin += spinVelocity * dt;
    }

    const floatWave = Math.sin((TAU * time) / cfg.floatPeriod);
    return {
      y: cfg.floatAmplitude * floatWave,
      floatWave,
      yaw: cfg.swayYaw * Math.sin((TAU * time) / cfg.swayYawPeriod) + pointer.x * cfg.pointerYaw,
      pitch: cfg.swayPitch * Math.sin((TAU * time) / cfg.swayPitchPeriod) + pointer.y * cfg.pointerPitch,
      spin,
    };
  }

  return {
    update,

    /** Ângulo atual de rotação em Y, em radianos (só leitura; usado nos testes). */
    get spin() {
      return spin;
    },

    /** x e y entre -1 e 1, relativos ao centro do palco. */
    setPointer(x, y) {
      pointerTarget.x = clamp(x, -1, 1);
      pointerTarget.y = clamp(y, -1, 1);
    },

    clearPointer() {
      pointerTarget.x = 0;
      pointerTarget.y = 0;
    },

    /** Uma volta completa e suave, terminando de frente. */
    spinOnce(duration) {
      if (dragging) return;
      const from = spin;
      const to = nearestRest(spin) + TAU;
      tween = { from, to, duration, elapsed: 0 };
      restAngle = to;
    },

    startDrag() {
      dragging = true;
      tween = null;
      dragStartSpin = spin;
      spinVelocity = 0;
    },

    /** turns: fração de volta desde o início do arrasto. dt em segundos. */
    moveDrag(turns, dt) {
      if (!dragging) return;
      const next = dragStartSpin + turns * TAU;
      // Eventos muito próximos dão velocidades absurdas: ignora-os e limita o resto.
      if (dt > 0.004) {
        const maxVelocity = cfg.maxSpinTurnsPerSecond * TAU;
        const instant = clamp((next - spin) / dt, -maxVelocity, maxVelocity);
        spinVelocity = spinVelocity * 0.6 + instant * 0.4;
      }
      spin = next;
    },

    endDrag() {
      if (!dragging) return;
      dragging = false;
      // Um lançamento forte dá no máximo mais uma volta antes de parar de frente.
      const reach = 0.75 * TAU;
      const projected = spin + clamp(spinVelocity * cfg.flickProjection, -reach, reach);
      restAngle = nearestRest(projected);
    },
  };
}

function nearestRest(angle) {
  return Math.round(angle / TAU) * TAU;
}

function easeInOutCubic(t) {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}
