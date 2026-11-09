// Only launches a prebuilt production server against an explicitly prepared
// disposable database. No local credentials, .env, migration or role bypass.
import { spawn } from 'node:child_process'
import { access } from 'node:fs/promises'
import { constants } from 'node:fs'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const root = fileURLToPath(new URL('../', import.meta.url))
function url(value, message) {
  try { return new URL(value) } catch { throw new Error(message) }
}
const origin = url(process.env.ADMIN_URL ?? 'http://127.0.0.1:5183', 'Admin E2E requires a valid loopback origin')
if (origin.protocol !== 'http:' || origin.hostname !== '127.0.0.1' ||
    !origin.port || origin.pathname !== '/' || origin.search || origin.hash ||
    origin.username || origin.password) {
  throw new Error('Admin E2E requires a plain 127.0.0.1 origin with an explicit port')
}
const configured = process.env.CLOUD_ADMIN_E2E_DATABASE_URL
if (!configured) throw new Error('Set CLOUD_ADMIN_E2E_DATABASE_URL to an explicitly migrated disposable loopback database')
const database = url(configured, 'Admin E2E requires a valid disposable loopback database URL')
if (!['postgres:', 'postgresql:'].includes(database.protocol) ||
    !['127.0.0.1', 'localhost', '[::1]'].includes(database.hostname) ||
    !/^\/cloud_admin_e2e(?:_[a-zA-Z0-9_-]+)?$/.test(database.pathname) ||
    database.search || database.hash) {
  throw new Error('Admin E2E refuses databases outside the loopback cloud_admin_e2e[_suffix] namespace')
}
const binary = path.resolve(process.env.CLOUD_ADMIN_E2E_BINARY ?? path.join(root, 'target/debug/cloud-api'))
const dist = path.join(root, 'web/admin/dist')
await Promise.all([access(binary, constants.X_OK), access(path.join(dist, 'index.html'), constants.R_OK)])

// Whitelist the environment so inherited Google, storage, Railway, proxy or
// operator configuration cannot turn this fixture into a live service.
const env = {
  PATH: process.env.PATH ?? '',
  DATABASE_URL: configured,
  AUTH_SECRET: 'admin-e2e-only-disposable-auth-secret-2026',
  APP_ORIGIN: origin.origin,
  BIND_ADDR: `127.0.0.1:${origin.port}`,
  ADMIN_DIST_DIR: dist,
  RUST_LOG: 'warn',
}
for (const key of ['LD_LIBRARY_PATH', 'DYLD_LIBRARY_PATH', 'SYSTEMROOT', 'WINDIR']) {
  if (process.env[key]) env[key] = process.env[key]
}
const server = spawn(binary, [], { cwd: root, env, stdio: 'inherit' })
server.on('error', (error) => { console.error(error.message); process.exitCode = 1 })
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => { server.kill(signal) })
server.on('exit', (code, signal) => { process.exitCode = code ?? (signal === 'SIGTERM' || signal === 'SIGINT' ? 0 : 1) })
