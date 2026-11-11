import { defineConfig } from 'vite'

export default defineConfig({
  root: './frontend/main',
  build: {
    outDir: '../../dist',
    emptyOutDir: true,
  },
})