import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createI18n } from 'vue-i18n'
import { createPinia, setActivePinia } from 'pinia'
import { VueQueryPlugin, QueryClient } from '@tanstack/vue-query'
import Portal from '../src/views/Portal.vue'
import { en } from '../src/i18n/en'
import { es } from '../src/i18n/es'

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
}

describe('publisher portal key gate', () => {
  it('renders key enrollment as disabled and never calls key endpoints', async () => {
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
    setActivePinia(createPinia())
    const wrapper = mount(Portal, {
      global: {
        plugins: [
          createI18n({ legacy: false, locale: 'en', messages: { en, es } }),
          [VueQueryPlugin, { queryClient: new QueryClient() }],
        ],
      },
    })
    await vi.waitFor(() => expect(wrapper.text()).toContain('acme'))
    expect(wrapper.text()).toMatch(/disabled|deshabilitada/i)
    expect(seen.some((u) => u.includes('/keys/'))).toBe(false)
    vi.unstubAllGlobals()
  })
})
