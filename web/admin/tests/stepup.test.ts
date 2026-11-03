import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createI18n } from 'vue-i18n'
import { ApiError, api } from '../src/api/client'
import { parseStepUpQuery, safeNextPath, stepUpReturnTo } from '../src/api/auth'
import StepUpDialog from '../src/components/StepUpDialog.vue'
import { en } from '../src/i18n/en'
import { es } from '../src/i18n/es'

function mockFetchOnce(body: unknown, status = 200) {
  const fetch = vi.fn().mockResolvedValue(
    new Response(body === undefined ? null : JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' },
    }),
  )
  vi.stubGlobal('fetch', fetch)
  return fetch
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('step-up query helpers', () => {
  it('parses the one-time grant or the opaque error from the callback query', () => {
    expect(parseStepUpQuery({ stepUpGrant: 'g1' })).toEqual({ grantId: 'g1' })
    expect(parseStepUpQuery({ stepUpError: 'stale_authentication' })).toEqual({ error: 'stale_authentication' })
    expect(parseStepUpQuery({})).toBeNull()
    expect(parseStepUpQuery({ stepUpGrant: '' })).toBeNull()
    expect(parseStepUpQuery({ stepUpGrant: ['g1'] })).toBeNull()
  })

  it('accepts only in-application return paths', () => {
    expect(stepUpReturnTo('/app/portal?stepUp=enroll')).toBe('/app/portal?stepUp=enroll')
    expect(stepUpReturnTo('/app')).toBe('/app')
    expect(stepUpReturnTo('https://evil.test/app')).toBeNull()
    expect(stepUpReturnTo('//evil.test/app')).toBeNull()
    expect(stepUpReturnTo('/app/portal#stepUpGrant=x')).toBeNull()
    expect(stepUpReturnTo('/login?next=/app')).toBeNull()
    expect(safeNextPath('/app/portal')).toBe('/app/portal')
    expect(safeNextPath('https://evil.test')).toBe('/app')
  })
})

describe('step-up API client', () => {
  it('posts the password grant request and surfaces backend error codes', async () => {
    const fetch = mockFetchOnce({ grantId: 'g1', expiresIn: 300 })
    const grant = await api.stepUpPassword('publisher.key.enroll', 'p1', null, 'secret password')
    expect(fetch.mock.calls[0]![0]).toBe('/api/v1/step-up/password')
    const body = JSON.parse((fetch.mock.calls[0]![1] as RequestInit).body as string)
    expect(body).toMatchObject({ action: 'publisher.key.enroll', publisherId: 'p1' })
    expect(grant.grantId).toBe('g1')
    mockFetchOnce({ error: 'invalid_credentials' }, 401)
    await expect(api.stepUpPassword('publisher.key.enroll', 'p1', null, 'nope')).rejects.toMatchObject({
      status: 401,
      code: 'invalid_credentials',
    })
    mockFetchOnce({ error: 'password_unavailable' }, 403)
    await expect(api.stepUpPassword('publisher.key.enroll', 'p1', null, 'nope')).rejects.toMatchObject({
      status: 403,
      code: 'password_unavailable',
    })
  })

  it('starts the Google flow and spends grants on key endpoints', async () => {
    mockFetchOnce({ authorizationUrl: 'https://provider.test/auth?state=s', expiresIn: 600 })
    const start = await api.stepUpStart('publisher.key.approve', 'p1', 'k1', '/app/publishers')
    expect(start.authorizationUrl).toContain('state=s')
    mockFetchOnce({ challengeId: 'c1', nonce: 'n', expiresIn: 300 })
    await api.publisherKeyChallenge('p1', 'g1')
    mockFetchOnce({ id: 'r1', keyId: 'k', publicKey: 'p', fingerprint: 'f', status: 'pending' }, 201)
    await api.requestPublisherKey('p1', { challengeId: 'c1', keyId: 'k', publicKey: 'p', signature: 's' }, 'g1')
    mockFetchOnce(undefined, 204)
    await api.approvePublisherKey('p1', 'r1', 'g1')
    mockFetchOnce({ error: 'step_up_expired' }, 403)
    const failure = await api.publisherKeyChallenge('p1', 'stale').catch((e: unknown) => e)
    expect(failure).toBeInstanceOf(ApiError)
    expect((failure as ApiError).code).toBe('step_up_expired')
  })

  it('never persists step-up material to web storage', () => {
    const sets: string[][] = []
    const spy = vi.spyOn(Storage.prototype, 'setItem').mockImplementation((k: string, v: string) => {
      sets.push([k, v])
    })
    expect(sets).toEqual([])
    spy.mockRestore()
  })
})

describe('StepUpDialog', () => {
  // reka-ui renders the dialog through a portal into document.body, so the
  // tests query there. Emitted events are still observed on the wrapper.
  function dialog(props: Record<string, unknown> = {}) {
    document.body.innerHTML = ''
    return mount(StepUpDialog, {
      props: {
        open: true,
        action: 'publisher.key.enroll',
        actionLabel: 'enroll a key',
        publisherId: 'p1',
        returnTo: '/app/portal',
        ...props,
      },
      global: {
        plugins: [createI18n({ legacy: false, locale: 'en', messages: { en, es } })],
      },
    })
  }

  function passwordInput(): HTMLInputElement {
    const input = document.body.querySelector('input[type="password"]')
    if (!input) throw new Error('password field is not rendered')
    return input as HTMLInputElement
  }

  async function typePassword(value: string) {
    // The portal content lands a tick after mount.
    await vi.waitFor(() => {
      if (!document.body.querySelector('input[type="password"]')) {
        throw new Error('waiting for the password field')
      }
    })
    const input = passwordInput()
    input.value = value
    input.dispatchEvent(new Event('input', { bubbles: true }))
  }

  async function submitPassword(value: string) {
    await typePassword(value)
    const form = document.body.querySelector('form')
    if (!form) throw new Error('password form is not rendered')
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  }

  function bodyText(): string {
    return document.body.textContent ?? ''
  }

  it('emits the grant on password success and clears the password', async () => {
    mockFetchOnce({ grantId: 'g1', expiresIn: 300 })
    const wrapper = dialog()
    await submitPassword('correct horse battery staple')
    await vi.waitFor(() => expect(wrapper.emitted('granted')).toEqual([['g1']]))
    // The password field is cleared after success.
    expect(passwordInput().value).toBe('')
  })

  it('shows invalid-password and unavailable states without leaking the password', async () => {
    mockFetchOnce({ error: 'invalid_credentials' }, 401)
    const wrapper = dialog()
    await submitPassword('wrong password here')
    await vi.waitFor(() => expect(bodyText()).toMatch(/Wrong password/i))
    expect(passwordInput().value).toBe('')
    mockFetchOnce({ error: 'password_unavailable' }, 403)
    await submitPassword('whatever password is fine')
    await vi.waitFor(() => expect(bodyText()).toMatch(/no password/i))
    expect(wrapper.emitted('granted')).toBeUndefined()
  })

  it('handles cancellation without contacting the backend', async () => {
    const fetch = mockFetchOnce({})
    const wrapper = dialog()
    await typePassword('typed but cancelled')
    const cancel = [...document.body.querySelectorAll('button')].find((b) =>
      (b.textContent ?? '').match(/Cancel/i),
    )
    if (!cancel) throw new Error('cancel button is not rendered')
    cancel.click()
    expect(wrapper.emitted('cancelled')).toHaveLength(1)
    expect(fetch).not.toHaveBeenCalled()
  })

  it('requests the Google authorization URL with the bound return path', async () => {
    const fetch = mockFetchOnce({ authorizationUrl: 'https://provider.test/auth?state=s', expiresIn: 600 })
    dialog({ action: 'publisher.key.approve', publisherId: 'p1', keyId: 'k1', returnTo: '/app/publishers?stepUp=approve' })
    await vi.waitFor(() => {
      if (!document.body.querySelector('button')) throw new Error('waiting for dialog buttons')
    })
    const google = [...document.body.querySelectorAll('button')].find((b) =>
      (b.textContent ?? '').match(/Google/i),
    )
    if (!google) throw new Error('google button is not rendered')
    google.click()
    await vi.waitFor(() => expect(fetch).toHaveBeenCalled())
    const body = JSON.parse((fetch.mock.calls[0]![1] as RequestInit).body as string)
    expect(body).toMatchObject({
      action: 'publisher.key.approve',
      publisherId: 'p1',
      keyId: 'k1',
      returnTo: '/app/publishers?stepUp=approve',
    })
  })
})
