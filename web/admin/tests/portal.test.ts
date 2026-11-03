import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createI18n } from 'vue-i18n'
import { createPinia, setActivePinia } from 'pinia'
import { VueQueryPlugin, QueryClient } from '@tanstack/vue-query'
import { createMemoryHistory, createRouter } from 'vue-router'
import Portal from '../src/views/Portal.vue'
import { en } from '../src/i18n/en'
import { es } from '../src/i18n/es'

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
}

function harness() {
  setActivePinia(createPinia())
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/app/portal', component: { template: '<div />' } }, { path: '/login', component: { template: '<div />' } }],
  })
  return router
}

describe('publisher portal step-up enrollment', () => {
  it('offers step-up enrollment without calling key endpoints upfront', async () => {
    const seen: string[] = []
    vi.stubGlobal(
      'fetch',
      vi.fn(async (url: string) => {
        seen.push(url)
        if (url === '/api/v1/publishers') {
          return json([{ id: 'p1', namespace: 'acme', status: 'approved', role: 'owner' }])
        }
        return json({ items: [], nextCursor: null })
      }),
    )
    const router = harness()
    await router.push('/app/portal')
    const wrapper = mount(Portal, {
      global: {
        plugins: [
          createI18n({ legacy: false, locale: 'en', messages: { en, es } }),
          [VueQueryPlugin, { queryClient: new QueryClient() }],
          router,
        ],
      },
    })
    await vi.waitFor(() => expect(wrapper.text()).toContain('acme'))
    expect(wrapper.text()).toMatch(/Enroll signing key|Inscribir clave/)
    // No key endpoint is touched before the user re-authenticates.
    expect(seen.some((u) => u.includes('/keys/'))).toBe(false)
    vi.unstubAllGlobals()
  })

  it('resumes the pending enrollment after the Google round-trip', async () => {
    const seen: string[] = []
    vi.stubGlobal(
      'fetch',
      vi.fn(async (url: string) => {
        seen.push(url)
        if (url === '/api/v1/publishers') {
          return json([{ id: 'p1', namespace: 'acme', status: 'approved', role: 'owner' }])
        }
        if (url === '/api/v1/publishers/p1/keys/challenges') {
          return json({ challengeId: 'c1', nonce: 'n'.repeat(64), expiresIn: 300 })
        }
        return json({ items: [], nextCursor: null })
      }),
    )
    const router = harness()
    await router.push('/app/portal?stepUp=enroll&publisher=p1&stepUpGrant=g1')
    const wrapper = mount(Portal, {
      global: {
        plugins: [
          createI18n({ legacy: false, locale: 'en', messages: { en, es } }),
          [VueQueryPlugin, { queryClient: new QueryClient() }],
          router,
        ],
      },
    })
    await vi.waitFor(() => expect(seen).toContain('/api/v1/publishers/p1/keys/challenges'))
    expect(wrapper.text()).toContain('c1')
    // The one-time grant is stripped from the URL once consumed.
    expect(router.currentRoute.value.query['stepUpGrant']).toBeUndefined()
    vi.unstubAllGlobals()
  })
})
