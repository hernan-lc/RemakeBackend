import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createI18n } from 'vue-i18n'
import { createPinia, setActivePinia } from 'pinia'
import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { createMemoryHistory, createRouter } from 'vue-router'
import Publishers from '../src/views/Publishers.vue'
import { en } from '../src/i18n/en'
import { es } from '../src/i18n/es'

afterEach(() => vi.unstubAllGlobals())

async function render(membershipStatus: number) {
  setActivePinia(createPinia())
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/app/publishers', component: Publishers }] })
  await router.push('/app/publishers')
  vi.stubGlobal('fetch', vi.fn(async (url: string) => {
    if (url === '/api/v1/admin/publishers/other/keys') {
      return new Response(JSON.stringify([{ id: 'k1', keyId: 'review-key', fingerprint: 'ab'.repeat(32), status: 'pending', publicKey: 'eA==' }]), { status: 200, headers: { 'Content-Type': 'application/json' } })
    }
    const membership = url === '/api/v1/publishers'
    const body = membership
      ? [{ id: 'own', namespace: 'own', status: 'pending', role: 'owner' }]
      : { contractVersion: 1, items: [
          { id: 'own', namespace: 'own', status: 'pending', createdAt: 'today' },
          { id: 'other', namespace: 'other', status: 'pending', createdAt: 'today' },
        ], nextCursor: null }
    return new Response(JSON.stringify(body), { status: membership ? membershipStatus : 200, headers: { 'Content-Type': 'application/json' } })
  }))
  return mount(Publishers, { global: { plugins: [
    createI18n({ legacy: false, locale: 'en', messages: { en, es } }),
    [VueQueryPlugin, { queryClient: new QueryClient({ defaultOptions: { queries: { retry: false } } }) }],
    router,
  ] } })
}

describe('publisher review eligibility', () => {
  it('explains the independent review requirement and allows other publishers', async () => {
    const wrapper = await render(200)
    await vi.waitFor(() => expect(wrapper.text()).toContain('different admin'))
    const rows = wrapper.findAll('tbody tr')
    expect(rows[0]!.find('button').attributes('disabled')).toBeDefined()
    expect(rows[1]!.find('button').attributes('disabled')).toBeUndefined()
    expect(rows[0]!.findAll('button')[1]!.attributes('disabled')).toBeDefined()
    wrapper.unmount()
  })

  it('fails closed when memberships cannot be loaded', async () => {
    const wrapper = await render(500)
    await vi.waitFor(() => expect(wrapper.text()).toContain('Could not check your publisher memberships'))
    expect(wrapper.find('tbody tr button').attributes('disabled')).toBeDefined()
    wrapper.unmount()
  })

  it('shows the complete fingerprint to an independent reviewer', async () => {
    const wrapper = await render(200)
    await vi.waitFor(() => expect(wrapper.text()).toContain('different admin'))
    const other = wrapper.findAll('tbody tr')[1]!
    await other.findAll('button').find((button) => button.text() === 'Keys')!.trigger('click')
    await vi.waitFor(() => expect(other.text()).toContain('ab'.repeat(32)))
    expect(other.text()).toContain('compare the full fingerprint')
    wrapper.unmount()
  })
})
