import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'
import { viteStaticCopy } from 'vite-plugin-static-copy'

export default defineConfig({
  root: './frontend/session',
  resolve: {
    alias: {
      '/shared': fileURLToPath(new URL('./frontend/shared', import.meta.url)),
    },
  },
  plugins: [
    viteStaticCopy({
      targets: [
        { src: fileURLToPath(new URL('./frontend/shared', import.meta.url)), dest: '.' },
        { src: fileURLToPath(new URL('./frontend/session/js', import.meta.url)), dest: '.' },
        { src: fileURLToPath(new URL('./frontend/main/icons', import.meta.url)), dest: '.' },
      ],
    }),
  ],
  build: {
    outDir: '../../dist/session',
    emptyOutDir: true,
    rollupOptions: {
      input: {
        session: fileURLToPath(new URL('./frontend/session/session.html', import.meta.url)),
        client: fileURLToPath(new URL('./frontend/session/client.html', import.meta.url)),
      },
    },
  },
})
