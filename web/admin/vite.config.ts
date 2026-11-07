import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'
import { demoPlugin } from './demo/plugin'

// Dev only: proxy same-origin `/api/*` to the Rust backend so cookies,
// redirects, and CSRF checks behave exactly like production behind the
// reverse proxy. No secret ever enters a Vite env var.
export default defineConfig(({ mode, command }) => {
  if (mode === 'demo' && command !== 'serve') throw new Error('Demo is development-only; use npm run build for production.')
  const demo = mode === 'demo'
  return {
    plugins: [vue(), tailwindcss(), ...(demo ? [demoPlugin()] : [])],
    server: {
      host: '127.0.0.1',
      port: 5173,
      strictPort: true,
      proxy: demo ? undefined : {
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
  }
})
