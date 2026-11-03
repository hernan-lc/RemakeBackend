import { defineConfig } from 'playwright/test'

// Cookie + CSRF integration against the real backend (and the dev proxy).
// Requires CLOUD_API_URL (default http://127.0.0.1:3000) serving the API and
// `npm run dev` on :5173. Not part of `npm test`.
export default defineConfig({
  testDir: './specs',
  timeout: 30_000,
  use: {
    baseURL: process.env.ADMIN_URL ?? 'http://127.0.0.1:5173',
  },
  webServer: undefined,
})
