// @vitest-environment node
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, beforeAll, expect, it } from 'vitest'
import { createServer, type ViteDevServer } from 'vite'
import { demoPlugin } from '../demo/plugin'
let server: ViteDevServer
let origin: string
let root: string
beforeAll(async () => {
  root = await mkdtemp(join(tmpdir(), 'remake-demo-test-'))
  server = await createServer({ root, configFile: false, plugins: [demoPlugin()], server: { host: '127.0.0.1', port: 0 } })
  await server.listen()
  const address = server.httpServer!.address() as { port: number }
  origin = `http://127.0.0.1:${address.port}`
})
afterAll(async () => { await server?.close(); await rm(root, { recursive: true, force: true }) })
it('provides fixtures and filters publishers without a backend', async () => {
  const me = await fetch(`${origin}/api/v1/me`).then(r => r.json())
  expect(me.email).toBe('demo@example.test')
  const page = await fetch(`${origin}/api/v1/admin/publishers?status=pending`).then(r => r.json())
  expect(page.items).toHaveLength(2)
})
it('fails closed for uploads, OAuth, step-up and unknown API paths', async () => {
  for (const path of ['/api/v1/plugins/uploads', '/api/auth/sign-in/social/google', '/api/v1/step-up/password', '/api/unknown']) {
    expect((await fetch(origin + path, { method: 'POST', body: '{}' })).status).toBe(501)
  }
})
it('keeps review separation and blocks cross-origin writes', async () => {
  expect((await fetch(`${origin}/api/v1/moderation/releases/release-0/decision`, { method: 'POST', body: JSON.stringify({decision:'approve'}) })).status).toBe(403)
  expect((await fetch(`${origin}/api/v1/admin/publishers/studio/approve`, { method: 'POST' })).status).toBe(403)
  expect((await fetch(`${origin}/api/v1/admin/publishers/music/approve`, { method: 'POST', headers: { Origin: 'https://other.test' } })).status).toBe(403)
  expect((await fetch(`${origin}/api/v1/admin/publishers/music/approve`, { method: 'POST' })).status).toBe(200)
  const data = await fetch(`${origin}/api/v1/admin/publishers?status=approved`).then(r => r.json())
  expect(data.items.some((p: { id: string }) => p.id === 'music')).toBe(true)
})
it('simulates logout and only accepts the documented demo credentials', async () => {
  const logout = await fetch(`${origin}/api/auth/sign-out`, { method: 'POST' })
  expect(logout.headers.get('set-cookie')).toContain('HttpOnly')
  expect((await fetch(`${origin}/api/v1/me`, { headers: { Cookie: 'demo_signed_out=1' } })).status).toBe(401)
  const signIn = (password: string) => fetch(`${origin}/api/auth/sign-in/email`, { method: 'POST', body: JSON.stringify({ email: 'demo@example.test', password }) })
  expect((await signIn('real-password')).status).toBe(401)
  expect((await signIn('demo-only')).status).toBe(200)
})
it('identifies the demo and does not reuse namespace IDs after deletion', async () => {
  const probe = await fetch(`${origin}/api/v1/auth/providers`)
  expect(probe.headers.get('X-Remake-Demo')).toBe('1')
  const create = (namespace: string) => fetch(`${origin}/api/v1/publishers`, { method: 'POST', body: JSON.stringify({namespace}) }).then(r => r.json())
  const first = await create('demo-first')
  const second = await create('demo-second')
  expect((await fetch(`${origin}/api/v1/publishers/${first.id}`, {method:'DELETE'})).status).toBe(200)
  const third = await create('demo-third')
  expect(new Set([first.id, second.id, third.id]).size).toBe(3)
})
