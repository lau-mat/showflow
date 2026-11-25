import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'
import { viteStaticCopy } from 'vite-plugin-static-copy'

export default defineConfig({
  root: './frontend/main',
  server: {
    proxy: {
      // The docker gateway (:8080) serves the API under /api, so the prefix is kept. API_PROXY_TARGET
      // points at a backend reached directly (`cargo run`), which has no /api prefix: strip it.
      '/api': {
        target: process.env.API_PROXY_TARGET ?? 'http://127.0.0.1:8080',
        changeOrigin: true,
        ...(process.env.API_PROXY_TARGET ? { rewrite: path => path.replace(/^\/api/, '') } : {}),
      },
    },
  },
  resolve: {
    alias: {
      '/shared': fileURLToPath(new URL('./frontend/shared', import.meta.url)),
    },
  },
  plugins: [
    viteStaticCopy({
      targets: [
        { src: fileURLToPath(new URL('./frontend/shared', import.meta.url)), dest: '.' },
        { src: fileURLToPath(new URL('./frontend/main/icons', import.meta.url)), dest: '.' },
        { src: fileURLToPath(new URL('./dist/session', import.meta.url)), dest: '.' },
        { src: fileURLToPath(new URL('./dist/session/assets', import.meta.url)), dest: '.' },
      ],
    }),
  ],
  build: {
    outDir: '../../dist/main',
    emptyOutDir: true,
  },
})