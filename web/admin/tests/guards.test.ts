import { describe, expect, it } from 'vitest'
import router from '../src/router'

describe('route guards are UX-only markers', () => {
  it('exposes public auth entry points', () => {
    expect(router.resolve('/login').meta.requiresAuth).toBeFalsy()
    expect(router.resolve('/auth/callback').meta.requiresAuth).toBeFalsy()
    expect(router.resolve('/403').meta.requiresAuth).toBeFalsy()
    expect(router.resolve('/logout').meta.requiresAuth).toBeFalsy()
  })

  it('marks the whole app shell as auth-gated', () => {
    for (const path of ['/app', '/app/publishers', '/app/moderation', '/app/audit', '/app/portal', '/app/account']) {
      const resolved = router.resolve(path)
      expect(resolved.matched.some((r) => r.meta.requiresAuth), path).toBe(true)
    }
  })

  it('redirects unknown paths into the shell', () => {
    // Redirects apply at navigation time, not in resolve(): pin the record.
    const catchAll = router.getRoutes().find((r) => r.path === '/:pathMatch(.*)*')
    expect(catchAll?.redirect).toBe('/app')
  })
})
