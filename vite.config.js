import { defineConfig } from 'vite'

export default defineConfig({
  root: './webapp', // Tells Vite: "Look inside the webapp folder for index.html"
  build: {
    outDir: '../dist', // Outputs the build back to root so Docker & Tauri find it easily
    emptyOutDir: true,
  },
})