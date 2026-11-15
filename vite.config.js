import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'
import { viteStaticCopy } from 'vite-plugin-static-copy'

export default defineConfig({
  root: './frontend/main',
  resolve: {
    alias: {
      '/shared': fileURLToPath(new URL('./frontend/shared', import.meta.url)),
    },
  },
  plugins: [
    viteStaticCopy({
      targets: [
        { src: fileURLToPath(new URL('./frontend/shared', import.meta.url)), dest: '.' },
        { src: fileURLToPath(new URL('./frontend/main/js/pages', import.meta.url)), dest: 'js' },
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