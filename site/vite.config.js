import { createHash } from 'node:crypto';
import { writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { defineConfig } from 'vite';

/**
 * Política de segurança de conteúdo (CSP) da página.
 *
 * Tudo vem do próprio site: sem CDNs, sem tipos de letra externos, sem analítica.
 * O único script embutido (o que escolhe o modo antes de a página aparecer, no
 * index.html) é autorizado pela sua impressão digital SHA-256, calculada aqui em
 * cada compilação. Não há 'unsafe-inline'.
 *
 * A compilação escreve a mesma política em dois sítios:
 *   - <meta> no index.html (funciona em qualquer alojamento);
 *   - dist/_headers (Cloudflare Workers e Pages, Netlify), que acrescenta frame-ancestors
 *     e outros cabeçalhos que não funcionam em <meta>.
 * O servidor de desenvolvimento (npm run dev) corre sem CSP.
 */
function policy(scriptHashes, { forHeaders = false } = {}) {
  const directives = [
    "default-src 'none'",
    `script-src 'self' ${scriptHashes.join(' ')}`.trim(),
    "style-src 'self'",
    "img-src 'self'",
    "font-src 'self'",
    "connect-src 'self'",
    "manifest-src 'self'",
    "base-uri 'none'",
    "form-action 'none'",
    "object-src 'none'",
  ];
  if (forHeaders) directives.push("frame-ancestors 'none'");
  return directives.join('; ');
}

function contentSecurityPolicy() {
  // Impressões digitais dos scripts embutidos de todas as páginas: o _headers vale para o
  // site inteiro, por isso tem de autorizar os scripts de cada página.
  const allHashes = new Set();
  return {
    name: 'sealer-csp',
    apply: 'build',
    transformIndexHtml: {
      order: 'post',
      handler(html) {
        const hashes = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map(
          (match) => `'sha256-${createHash('sha256').update(match[1]).digest('base64')}'`,
        );
        for (const hash of hashes) allHashes.add(hash);
        const meta = `<meta http-equiv="Content-Security-Policy" content="${policy(hashes)}">`;
        return html.replace('<head>', `<head>\n    ${meta}`);
      },
    },
    writeBundle(options) {
      const headersPolicy = policy([...allHashes].sort(), { forHeaders: true });
      const headers = [
        '# Gerado por vite.config.js em cada compilação. Não editar à mão.',
        '# Cabeçalhos de segurança para a Cloudflare (Workers e Pages) e a Netlify.',
        '# Noutros servidores (nginx, Apache, Caddy), copie estes valores; ver README.md.',
        '/*',
        `  Content-Security-Policy: ${headersPolicy}`,
        '  Referrer-Policy: no-referrer',
        '  X-Content-Type-Options: nosniff',
        '  Permissions-Policy: camera=(), microphone=(), geolocation=(), interest-cohort=()',
        '  Cross-Origin-Opener-Policy: same-origin',
        '',
        '/assets/*',
        '  Cache-Control: public, max-age=31536000, immutable',
        '',
      ].join('\n');
      writeFileSync(join(options.dir, '_headers'), headers);
    },
  };
}

export default defineConfig({
  // Caminhos relativos: a pasta dist/ funciona na raiz do domínio ou numa subpasta.
  base: './',
  plugins: [contentSecurityPolicy()],
  build: {
    target: 'es2020',
    // Nada embutido como data: URI, para a CSP poder ficar em img-src 'self'.
    assetsInlineLimit: 0,
    // O Three.js (cerca de 540 KB, 134 KB comprimido) fica num ficheiro próprio,
    // pedido só depois de a página carregar. O aviso de 500 KB não se aplica.
    chunkSizeWarningLimit: 600,
    rollupOptions: {
      // Páginas do site. Uma página nova tem de ser acrescentada aqui.
      input: {
        main: resolve(import.meta.dirname, 'index.html'),
        cookies: resolve(import.meta.dirname, 'politica-de-cookies.html'),
        comoFunciona: resolve(import.meta.dirname, 'como-funciona.html'),
      },
      output: {
        manualChunks(id) {
          if (id.includes('node_modules/three')) return 'three';
        },
      },
    },
  },
});
