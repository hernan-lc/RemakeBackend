import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createI18n } from 'vue-i18n'
import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { createMemoryHistory, createRouter } from 'vue-router'
import DocumentModeration from '../src/views/DocumentModeration.vue'
import ConfirmDialog from '../src/components/ConfirmDialog.vue'
import { en } from '../src/i18n/en'
import { es } from '../src/i18n/es'

const id = 'e2f5fa3e-7e39-4c5a-9b88-2d062d70728f'
const sha256 = 'a'.repeat(64)
const pending = { id, kind: 'template', slug: 'greeting', revision: 2, title: 'Greeting', sha256, status: 'pending', document: { title: '<img src=x onerror=alert(1)>', actions: [{ type: 'log' }] } }
const published = { ...pending, status: 'published' }
const session = { id: 'reviewer', email: 'reviewer@example.test', name: 'Reviewer', roles: ['moderator'] }
const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { 'Content-Type': 'application/json' } })

afterEach(() => { vi.unstubAllGlobals(); document.body.innerHTML = '' })

async function render(overrides: { forbidden?: boolean; decisionFails?: boolean; mismatch?: boolean } = {}) {
  const fetcher = vi.fn(async (url: string, init?: RequestInit) => {
    if (url === '/api/v1/me') return json(session)
    if (url === '/api/v2/moderation/documents') return overrides.forbidden ? json({}, 403) : json({ items: [pending], nextCursor: null })
    if (url === '/api/v2/catalog/documents') return json({ items: [published], nextCursor: null })
    if (url === `/api/v2/catalog/documents/${id}/versions/2`) return json(overrides.mismatch ? { ...published, sha256: 'b'.repeat(64) } : published)
    if (url.startsWith(`/api/v2/moderation/documents/${id}/versions/2/`) && init?.method === 'POST') return new Response(null, { status: overrides.decisionFails ? 409 : 204 })
    throw new Error(`unexpected request ${url}`)
  })
  vi.stubGlobal('fetch', fetcher)
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/app/moderation/documents', component: DocumentModeration }, { path: '/app/moderation', component: { template: '<div />' } }] })
  await router.push('/app/moderation/documents')
  const wrapper = mount(DocumentModeration, { attachTo: document.body, global: { plugins: [
    createI18n({ legacy: false, locale: 'en', messages: { en, es } }),
    [VueQueryPlugin, { queryClient: new QueryClient({ defaultOptions: { queries: { retry: false } } }) }],
    router,
  ] } })
  return { wrapper, fetcher }
}

const button = (wrapper: ReturnType<typeof mount>, text: string) => wrapper.findAll('button').find((item) => item.text() === text)!

describe('Cloud document moderation', () => {
  it('shows escaped source and submits an exact reviewed revision with a bounded reason', async () => {
    const { wrapper, fetcher } = await render()
    await vi.waitFor(() => expect(wrapper.find('[data-testid="document-content"]').text()).toContain('<img src=x'))
    expect(wrapper.find('[data-testid="document-content"] img').exists()).toBe(false)
    await button(wrapper, 'Approve').trigger('click')
    const dialog = wrapper.findComponent(ConfirmDialog)
    expect(dialog.props('body')).toContain(sha256)
    dialog.vm.$emit('update:promptValue', '😀'.repeat(126))
    await wrapper.vm.$nextTick()
    expect(dialog.props('confirmDisabled')).toBe(true)
    dialog.vm.$emit('confirm')
    expect(fetcher.mock.calls.filter(([url]) => url.endsWith('/decision'))).toHaveLength(0)
    dialog.vm.$emit('update:promptValue', 'Reviewed the actions')
    await wrapper.vm.$nextTick()
    dialog.vm.$emit('confirm')
    await vi.waitFor(() => expect(fetcher.mock.calls.filter(([url]) => url.endsWith('/decision'))).toHaveLength(1))
    const [, init] = fetcher.mock.calls.find(([url]) => url.endsWith('/decision'))!
    expect(JSON.parse(String(init?.body))).toEqual({ decision: 'approve', reason: 'Reviewed the actions' })
    expect(init?.credentials).toBe('include')
    wrapper.unmount()
  })

  it('does not expose review actions when the server denies access', async () => {
    const { wrapper } = await render({ forbidden: true })
    await vi.waitFor(() => expect(wrapper.text()).toContain('cannot moderate'))
    expect(wrapper.find('[data-testid="document-content"]').exists()).toBe(false)
    expect(wrapper.findAll('button').some((item) => item.text() === 'Approve')).toBe(false)
    wrapper.unmount()
  })

  it('requires the exact published content identity before enabling withdrawal', async () => {
    const { wrapper } = await render({ mismatch: true })
    await vi.waitFor(() => expect(wrapper.text()).toContain('Greeting'))
    await button(wrapper, 'Published').trigger('click')
    await vi.waitFor(() => expect(wrapper.text()).toContain('could not be verified'))
    expect(wrapper.findAll('button').some((item) => item.text() === 'Withdraw publication')).toBe(false)
    wrapper.unmount()
  })

  it('withdraws the displayed published revision with its review reason', async () => {
    const { wrapper, fetcher } = await render()
    await vi.waitFor(() => expect(wrapper.text()).toContain('Greeting'))
    await button(wrapper, 'Published').trigger('click')
    await vi.waitFor(() => expect(wrapper.findAll('button').some((item) => item.text() === 'Withdraw publication')).toBe(true))
    await button(wrapper, 'Withdraw publication').trigger('click')
    const dialog = wrapper.findComponent(ConfirmDialog)
    expect(dialog.props('body')).toContain(sha256)
    dialog.vm.$emit('update:promptValue', 'Unsafe instructions')
    await wrapper.vm.$nextTick()
    dialog.vm.$emit('confirm')
    await vi.waitFor(() => expect(fetcher.mock.calls.filter(([url]) => url.endsWith('/withdraw'))).toHaveLength(1))
    const [, init] = fetcher.mock.calls.find(([url]) => url.endsWith('/withdraw'))!
    expect(JSON.parse(String(init?.body))).toEqual({ reason: 'Unsafe instructions' })
    wrapper.unmount()
  })

  it('reports an uncertain decision without automatically repeating the mutation', async () => {
    const { wrapper, fetcher } = await render({ decisionFails: true })
    await vi.waitFor(() => expect(wrapper.text()).toContain('Greeting'))
    await button(wrapper, 'Reject').trigger('click')
    const dialog = wrapper.findComponent(ConfirmDialog)
    dialog.vm.$emit('update:promptValue', 'Needs revision')
    await wrapper.vm.$nextTick()
    dialog.vm.$emit('confirm')
    await vi.waitFor(() => expect(wrapper.text()).toContain('result could not be confirmed'))
    expect(fetcher.mock.calls.filter(([url]) => url.endsWith('/decision'))).toHaveLength(1)
    wrapper.unmount()
  })
})
