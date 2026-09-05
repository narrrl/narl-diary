import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

export default defineConfig({
  plugins: [svelte()],
  build: { target: 'es2022', chunkSizeWarningLimit: 900 },
  server: {
    port: 4243,
    proxy: {
      '/api': { target: 'http://127.0.0.1:4242', changeOrigin: false },
    },
  },
})
