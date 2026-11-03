import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

// Dev only: proxy same-origin `/api/*` to the Rust backend so cookies,
// redirects, and CSRF checks behave exactly like production behind the
// reverse proxy. No secret ever enters a Vite env var.
export default defineConfig({
  plugins: [vue(), tailwindcss()],
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
    proxy: {
      '/api': {
        target: process.env.CLOUD_API_URL ?? 'http://127.0.0.1:3000',
        changeOrigin: false,
      },
    },
  },
  preview: {
    port: 4173,
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./tests/setup.ts'],
    exclude: ['playwright/**', 'node_modules/**'],
  },
})
