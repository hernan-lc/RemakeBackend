import { afterEach, describe, expect, it, vi } from 'vitest'
import { ApiError, api } from '../src/api/client'

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

describe('typed API client', () => {
  it('sends cookie sessions, never tokens', async () => {
    const fetch = mockFetchOnce({ id: 'u', email: 'a@b.c', name: 'n', roles: ['admin'] })
    await api.me()
    const [url, init] = fetch.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('/api/v1/me')
    expect(init.credentials).toBe('include')
    expect(JSON.stringify(init.headers)).not.toMatch(/bearer|token|authoriz/i)
  })

  it('maps HTTP failures to statuses for 401/403 handling', async () => {
    mockFetchOnce({ error: 'x' }, 401)
    await expect(api.overview()).rejects.toMatchObject({ status: 401 })
    mockFetchOnce({ error: 'x' }, 403)
    await expect(api.audit({})).rejects.toMatchObject({ status: 403 })
  })

  it('builds admin list query strings', async () => {
    const fetch = mockFetchOnce({ contractVersion: 1, items: [], nextCursor: null })
    await api.adminPublishers({ status: 'pending', cursor: 'abc', limit: 50 })
    expect(fetch.mock.calls[0]![0]).toBe('/api/v1/admin/publishers?status=pending&cursor=abc&limit=50')
  })

  it('never persists anything to web storage', async () => {
    // jsdom provides storage; the assertion is that the client never uses
    // it. Any token-in-storage regression trips these spies.
    const sets: string[][] = []
    const spy = vi.spyOn(Storage.prototype, 'setItem').mockImplementation((k: string, v: string) => {
      sets.push([k, v])
    })
    mockFetchOnce({ id: 'u', email: 'a@b.c', name: 'n', roles: [] })
    await api.me()
    mockFetchOnce(undefined, 204)
    await api.signOut()
    expect(sets).toEqual([])
    spy.mockRestore()
    expect(ApiError).toBeDefined()
  })
})
