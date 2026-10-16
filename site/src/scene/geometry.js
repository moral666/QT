import { BufferGeometry, Float32BufferAttribute } from 'three';

/**
 * Cria as paredes laterais do logótipo a partir do contorno 2D.
 *
 * Em vez de usar ExtrudeGeometry (que gera normais facetadas e tampas que
 * não precisamos), constrói uma faixa fechada à volta do contorno com
 * vértices partilhados e normais suaves. As faces da frente e de trás são
 * planos com a imagem do logótipo, desenhados à parte.
 *
 * @param {number[][]} points Contorno fechado, sentido anti-horário, em unidades da imagem.
 * @param {object} options
 * @param {number} options.depth Espessura total.
 * @param {number} options.bulge Quanto a parede incha ao meio.
 * @param {number} options.segments Subdivisões do perfil.
 * @param {number} options.normalBend Inclinação das normais nas arestas, em radianos.
 */
export function createWallGeometry(points, { depth, bulge, segments, normalBend }) {
  const count = points.length;
  const outward = outwardNormals(points);
  const rings = segments + 1;

  const positions = new Float32Array(count * rings * 3);
  const normals = new Float32Array(count * rings * 3);

  for (let r = 0; r < rings; r++) {
    const s = r / segments; // 0 = frente, 1 = trás
    const z = (depth / 2) * (1 - 2 * s);
    const offset = bulge * Math.sin(Math.PI * s);
    const phi = (0.5 - s) * 2 * normalBend;
    const nz = Math.sin(phi);
    const nxy = Math.cos(phi);

    for (let i = 0; i < count; i++) {
      const k = (r * count + i) * 3;
      const [px, py] = points[i];
      const [nx, ny] = outward[i];
      positions[k] = px + nx * offset;
      positions[k + 1] = py + ny * offset;
      positions[k + 2] = z;
      normals[k] = nx * nxy;
      normals[k + 1] = ny * nxy;
      normals[k + 2] = nz;
    }
  }

  const indices = [];
  for (let r = 0; r < segments; r++) {
    for (let i = 0; i < count; i++) {
      const a = r * count + i;
      const b = r * count + ((i + 1) % count);
      const c = (r + 1) * count + i;
      const d = (r + 1) * count + ((i + 1) % count);
      // Ordem anti-horária vista de fora, para o lado da frente apontar para fora.
      indices.push(a, c, b, b, c, d);
    }
  }

  const geometry = new BufferGeometry();
  geometry.setAttribute('position', new Float32BufferAttribute(positions, 3));
  geometry.setAttribute('normal', new Float32BufferAttribute(normals, 3));
  geometry.setIndex(indices);
  geometry.computeBoundingSphere();
  return geometry;
}

/** Normal exterior em cada vértice de um polígono anti-horário (média das duas arestas). */
function outwardNormals(points) {
  const count = points.length;
  const result = new Array(count);
  for (let i = 0; i < count; i++) {
    const prev = points[(i - 1 + count) % count];
    const curr = points[i];
    const next = points[(i + 1) % count];
    const [ax, ay] = edgeNormal(prev, curr);
    const [bx, by] = edgeNormal(curr, next);
    const nx = ax + bx;
    const ny = ay + by;
    const length = Math.hypot(nx, ny) || 1;
    result[i] = [nx / length, ny / length];
  }
  return result;
}

function edgeNormal(from, to) {
  const dx = to[0] - from[0];
  const dy = to[1] - from[1];
  const length = Math.hypot(dx, dy) || 1;
  return [dy / length, -dx / length];
}
