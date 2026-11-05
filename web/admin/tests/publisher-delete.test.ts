import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createI18n } from 'vue-i18n'
import { createPinia, setActivePinia } from 'pinia'
import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { createMemoryHistory, createRouter } from 'vue-router'
import Portal from '../src/views/Portal.vue'
import { en } from '../src/i18n/en'
import { es } from '../src/i18n/es'

afterEach(() => {
  vi.unstubAllGlobals()
  document.body.innerHTML = ''
})

async function setup(deleteStatus: number) {
  setActivePinia(createPinia())
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/app/portal', component: Portal }] })
  await router.push('/app/portal')
  let removed = false
  const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
    if (init?.method === 'DELETE') {
      removed = deleteStatus === 204
      return new Response(null, { status: deleteStatus })
    }
    const list = [
      ...(!removed ? [{ id: 'owner-id', namespace: 'owner-space', status: 'pending', role: 'owner' }] : []),
      { id: 'viewer-id', namespace: 'viewer-space', status: 'pending', role: 'viewer' },
    ]
    return new Response(JSON.stringify(list), { status: 200, headers: { 'Content-Type': 'application/json' } })
  })
  vi.stubGlobal('fetch', fetchMock)
  const wrapper = mount(Portal, { attachTo: document.body, global: { plugins: [
    createI18n({ legacy: false, locale: 'en', messages: { en, es } }),
    [VueQueryPlugin, { queryClient: new QueryClient({ defaultOptions: { queries: { retry: false } } }) }],
    router,
  ] } })
  await vi.waitFor(() => expect(wrapper.text()).toContain('owner-space'))
  return { wrapper, fetchMock }
}

async function confirmDelete(wrapper: ReturnType<typeof mount>) {
  const buttons = wrapper.findAll('button').filter((button) => button.text() === 'Delete namespace')
  expect(buttons).toHaveLength(1)
  await buttons[0]!.trigger('click')
  await vi.waitFor(() => expect(document.body.textContent).toContain('Type the namespace to confirm'))
  const dialog = document.querySelector('[role="dialog"]')!
  const confirm = [...dialog.querySelectorAll('button')].find((button) => button.textContent?.trim() === 'Confirm')!
  expect(confirm.disabled).toBe(true)
  const input = dialog.querySelector('input')!
  input.value = 'owner-space'
  input.dispatchEvent(new Event('input', { bubbles: true }))
  await vi.waitFor(() => expect(confirm.disabled).toBe(false))
  confirm.click()
}

describe('publisher namespace deletion', () => {
  it('deletes only after the owner confirms the exact namespace', async () => {
    const { wrapper, fetchMock } = await setup(204)
    expect(wrapper.text()).toContain('Only approved publishers accept releases.')
    expect(wrapper.text()).not.toContain('Loading…')
    await confirmDelete(wrapper)
    await vi.waitFor(() => expect(wrapper.text()).not.toContain('owner-space'))
    expect(fetchMock).toHaveBeenCalledWith('/api/v1/publishers/owner-id', expect.objectContaining({ method: 'DELETE' }))
    expect(wrapper.text()).toContain('viewer-space')
    wrapper.unmount()
  })

  it('keeps the dialog open and explains a conflict', async () => {
    const { wrapper } = await setup(409)
    await confirmDelete(wrapper)
    await vi.waitFor(() => expect(document.body.textContent).toContain('cannot be deleted while it has other members'))
    expect(wrapper.text()).toContain('owner-space')
    wrapper.unmount()
  })
})
