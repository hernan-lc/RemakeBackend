import type { IncomingMessage, ServerResponse } from 'node:http'
type DemoServer = { middlewares: { use(handler: (req: IncomingMessage, res: ServerResponse, next: () => void) => void): unknown } }
import type { MyPublisher, PublisherKey, PublisherRelease, PublisherReviewItem, AuditEventView, Overview } from '../src/api/types'

// Server-only fixtures: never imported by the application or production bundle.
export function demoPlugin() {
  let nextPublisherId = 1
  const date = '2026-11-06T14:30:00Z'
  const publishers: (PublisherReviewItem & { role?: string })[] = [
    { id: 'studio', namespace: 'lumen-studio', status: 'approved', role: 'owner', createdAt: date },
    { id: 'music', namespace: 'sound-lab', status: 'pending', createdAt: date },
    { id: 'overlay', namespace: 'overlay-kit', status: 'pending', createdAt: date },
    { id: 'archive', namespace: 'legacy-tools', status: 'suspended', createdAt: date },
  ]
  const keys = (id: string): PublisherKey[] => [{ id: `key-${id}`, keyId: 'demo-signing-key', publicKey: 'DEMO-NOT-A-REAL-KEY', fingerprint: 'ab'.repeat(32), status: id === 'studio' ? 'approved' : 'pending' }]
  const releases: PublisherRelease[] = ['chat-overlay', 'stream-alerts', 'scene-switcher'].map((pluginId, i) => ({
    id: `release-${i}`, publisherId: i === 0 ? 'studio' : i === 1 ? 'music' : 'overlay', uploadId: `upload-${i}`, publisherKeyId: 'key-studio', pluginId,
    version: `1.${i}.0`, target: 'desktop', status: 'antivirus_passed', sha256: 'cd'.repeat(32), size: 240000 + i * 17000,
    keyId: 'demo-signing-key', keyFingerprint: 'ab'.repeat(32), yanked: false, yankReason: null,
    advisory: null, moderation: null, verification: { status: 'passed', resultClass: 'clean' },
  }))
  const events: AuditEventView[] = ['publisher.created', 'publisher.approved', 'release.submitted', 'key.approved'].map((action, i) => ({
    id: `event-${i}`, action, actorUserId: 'demo-reviewer', targetUserId: null, publisherId: 'studio', subjectId: `subject-${i}`, oldRole: null, newRole: null, createdAt: date,
  }))
  return {
    name: 'isolated-local-demo',
    apply: 'serve' as const,
    transformIndexHtml: () => [{ tag: 'aside', attrs: { style: 'position:relative;z-index:100;background:#fef3c7;color:#78350f;padding:10px 20px;font:13px system-ui;text-align:center', role: 'note' }, children: 'DEMO · Datos ficticios / Fictional data · demo@example.test / demo-only · No uses credenciales reales. OAuth, claves, archivos y transferencias no se ejecutan.', injectTo: 'body-prepend' as const }],
    configureServer(server: DemoServer) {
      server.middlewares.use(async (req, res, next) => {
        const url = new URL(req.url ?? '/', 'http://localhost')
        if (!url.pathname.startsWith('/api/')) return next()
        // Every API route terminates here; unknown paths NEVER reach a proxy.
        res.setHeader('X-Remake-Demo', '1')
        res.setHeader('Content-Type', 'application/json')
        res.setHeader('Cache-Control', 'no-store')
        const send = (status: number, data: unknown) => { res.statusCode = status; res.end(JSON.stringify(data)) }
        const method = req.method ?? 'GET'
        if (!['GET', 'HEAD'].includes(method) && req.headers.origin && req.headers.origin !== `http://${req.headers.host}`) return send(403, { error: 'demo_origin_rejected' })
        const path = url.pathname
        if (path === '/api/v1/auth/providers') return send(200, { google: false })
        if (path === '/api/auth/sign-out' && method === 'POST') {
          res.setHeader('Set-Cookie', 'demo_signed_out=1; HttpOnly; SameSite=Strict; Path=/')
          return send(200, {})
        }
        let body: Record<string, unknown> = {}
        if (!['GET', 'HEAD'].includes(method)) {
          try {
            let raw = ''
            for await (const chunk of req) { raw += chunk; if (raw.length > 16384) return send(413, { error: 'demo_body_too_large' }) }
            body = raw ? JSON.parse(raw) : {}
            if (!body || typeof body !== 'object' || Array.isArray(body)) return send(400, { error: 'invalid_body' })
          } catch { return send(400, { error: 'invalid_json' }) }
        }
        if (/^\/api\/auth\/sign-in\/(email|username)$/.test(path) && method === 'POST') {
          if ((body.email ?? body.username) !== 'demo@example.test' || body.password !== 'demo-only') return send(401, { error: 'demo_credentials_only' })
          res.setHeader('Set-Cookie', 'demo_signed_out=; Max-Age=0; HttpOnly; SameSite=Strict; Path=/')
          return send(200, {})
        }
        if (req.headers.cookie?.split(';').some(c => c.trim() === 'demo_signed_out=1')) return send(401, { error: 'demo_signed_out' })
        if (method === 'GET') {
          if (path === '/api/v1/me') return send(200, { id: 'demo-reviewer', email: 'demo@example.test', name: 'Alex Demo', roles: ['admin', 'moderator'] })
          if (path === '/api/v1/me/providers') return send(200, [{ providerId: 'credential', accountId: 'demo@example.test' }])
          if (path === '/api/v1/admin/temporary-log-approvals') return send(200, { enabled: false })
          if (path === '/api/v1/admin/overview') {
            const overview: Overview = { contractVersion: 1, publishers: { pending: publishers.filter(p => p.status === 'pending').length, approved: publishers.filter(p => p.status === 'approved').length, suspended: publishers.filter(p => p.status === 'suspended').length }, releases: { awaitingVerification: 0, integrityPassed: 0, antivirusPassed: releases.filter(r => !r.moderation).length, reviewApproved: releases.filter(r => r.moderation?.decision === 'approve').length, reviewRejected: releases.filter(r => r.moderation?.decision === 'reject').length, rejected: 0 }, pendingKeys: 2, auditEvents: events.length }
            return send(200, overview)
          }
          if (path === '/api/v1/admin/publishers') return send(200, { contractVersion: 1, items: publishers.filter(p => !url.searchParams.get('status') || p.status === url.searchParams.get('status')), nextCursor: null })
          if (path === '/api/v1/admin/audit') return send(200, { contractVersion: 1, events, nextCursor: null })
          if (path === '/api/v1/moderation/releases') return send(200, { items: releases.filter(r => !r.moderation && r.publisherId !== 'studio').map(r => ({ ...r, namespace: publishers.find(p => p.id === r.publisherId)?.namespace })), nextCursor: null })
          if (path === '/api/v1/publishers') return send(200, publishers.filter(p => p.role).map(p => ({ id: p.id, namespace: p.namespace, status: p.status, role: p.role! } satisfies MyPublisher)))
          const sub = path.match(/^\/api\/v1\/(?:admin\/)?publishers\/([^/]+)\/(keys|members|releases|transfer)$/)
          if (sub) {
            if (!publishers.some(p => p.id === sub[1])) return send(404, { error: 'not_found' })
            if (sub[2] === 'keys') return send(200, keys(sub[1]!))
            if (sub[2] === 'members') return send(200, [{ user_id: 'demo-reviewer', role: 'owner' }, { user_id: 'demo-editor', role: 'publisher' }])
            if (sub[2] === 'releases') return send(200, { items: releases.filter(r => r.publisherId === sub[1]), nextCursor: null })
            return send(404, { error: 'no_transfer' })
          }
          const review = path.match(/^\/api\/v1\/plugins\/releases\/([^/]+)\/review$/)
          if (review) return send(200, { decision: 'pending', reasonCode: '', yanked: false, yankReason: null, advisory: null })
        }
        if (method === 'POST' && path === '/api/v1/publishers') {
          const namespace = typeof body.namespace === 'string' ? body.namespace : ''
          if (!/^[a-z][a-z0-9-]{0,61}[a-z0-9]$/.test(namespace)) return send(400, { error: 'invalid_namespace' })
          if (publishers.some(p => p.namespace === namespace)) return send(409, { error: 'namespace_taken' })
          const p = { id: `demo-${nextPublisherId++}`, namespace, status: 'pending', role: 'owner', createdAt: date }
          publishers.push(p); return send(200, p)
        }
        const approval = path.match(/^\/api\/v1\/admin\/publishers\/([^/]+)\/approve$/)
        if (method === 'POST' && approval) {
          const p = publishers.find(p => p.id === approval[1])
          if (!p) return send(404, { error: 'not_found' })
          if (p.role) return send(403, { error: 'independent_reviewer_required' })
          p.status = 'approved'; return send(200, {})
        }
        const deletion = path.match(/^\/api\/v1\/publishers\/([^/]+)$/)
        if (method === 'DELETE' && deletion) {
          const index = publishers.findIndex(p => p.id === deletion[1])
          if (index < 0) return send(404, { error: 'not_found' })
          if (!publishers[index]?.role) return send(403, { error: 'forbidden' })
          if (deletion[1] === 'studio') return send(409, { error: 'publisher_not_empty' })
          publishers.splice(index, 1); return send(200, {})
        }
        const decision = path.match(/^\/api\/v1\/moderation\/releases\/([^/]+)\/decision$/)
        if (method === 'POST' && decision) {
          const release = releases.find(r => r.id === decision[1])
          if (!release) return send(404, { error: 'not_found' })
          if (publishers.find(p => p.id === release.publisherId)?.role) return send(403, { error: 'independent_reviewer_required' })
          if (!['approve', 'reject'].includes(String(body.decision))) return send(400, { error: 'invalid_decision' })
          release.moderation = { decision: String(body.decision), reasonCode: String(body.reasonCode ?? '') }; return send(200, {})
        }
        // Security-sensitive and unsupported workflows fail explicitly, never fake success.
        return send(501, { error: 'demo_operation_unavailable' })
      })
    },
  }
}
