import { defineConfig } from 'playwright/test'
import { fileURLToPath } from 'node:url'

// The ordinary Rust binary serves the production UI build and real APIs.
// Migrate a dedicated loopback cloud_admin_e2e[_suffix] database explicitly
// before running. The launcher never reads .env or runs migrations.
let origin: string
try { origin = new URL(process.env.ADMIN_URL ?? 'http://127.0.0.1:5183').origin }
catch { throw new Error('Admin E2E requires a valid loopback origin') }
export default defineConfig({
  testDir: './specs',
  timeout: 30_000,
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: 0,
  outputDir: process.env.CLOUD_ADMIN_E2E_OUTPUT_DIR ?? '../test-results',
  reporter: [['list']],
  use: {
    baseURL: origin,
    browserName: 'chromium',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  webServer: {
    command: 'node ../../scripts/admin-e2e-server.mjs',
    cwd: fileURLToPath(new URL('..', import.meta.url)),
    url: `${origin}/health/ready`,
    timeout: 30_000,
    reuseExistingServer: false,
    stdout: 'pipe',
    stderr: 'pipe',
  },
})
