import { afterEach, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createI18n } from 'vue-i18n'
import TemporaryApprovalDialog from '../src/components/TemporaryApprovalDialog.vue'
import { en } from '../src/i18n/en'
import { es } from '../src/i18n/es'

afterEach(() => { vi.unstubAllGlobals(); document.body.innerHTML = '' })

it('requests a scoped code and submits eight digits only to the confirmation endpoint', async () => {
  const fetchMock = vi.fn(async (url: string) => new Response(
    url.endsWith('/confirm') ? null : JSON.stringify({ challengeId: 'challenge-1', expiresIn: 300, fingerprint: 'ab'.repeat(32) }),
    { status: url.endsWith('/confirm') ? 204 : 200, headers: { 'Content-Type': 'application/json' } },
  ))
  vi.stubGlobal('fetch', fetchMock)
  const wrapper = mount(TemporaryApprovalDialog, {
    attachTo: document.body,
    props: { open: true, publisherId: 'p1', namespace: 'tiktools', keyId: 'k1' },
    global: { plugins: [createI18n({ legacy: false, locale: 'en', messages: { en, es } })] },
  })
  await vi.waitFor(() => expect(document.body.textContent).toContain('Generate code'))
  const click = (label: string) => [...document.querySelectorAll('button')].find((button) => button.textContent?.trim() === label)!.click()
  click('Generate code')
  await vi.waitFor(() => expect(document.body.textContent).toContain('challenge-1'))
  expect(document.body.textContent).toContain('ab'.repeat(32))
  const input = document.querySelector('input')!
  input.value = '12345678'
  input.dispatchEvent(new Event('input', { bubbles: true }))
  await vi.waitFor(() => expect([...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Confirm')!.disabled).toBe(false))
  click('Confirm')
  await vi.waitFor(() => expect(wrapper.emitted('approved')).toBeDefined())
  expect(fetchMock).toHaveBeenCalledWith('/api/v1/admin/temporary-log-approvals', expect.objectContaining({ body: JSON.stringify({ publisherId: 'p1', keyId: 'k1' }) }))
  expect(fetchMock).toHaveBeenCalledWith('/api/v1/admin/temporary-log-approvals/challenge-1/confirm', expect.objectContaining({ body: JSON.stringify({ code: '12345678' }) }))
  wrapper.unmount()
})
