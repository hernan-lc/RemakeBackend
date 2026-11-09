import { expect, test } from 'playwright/test'
import { randomUUID } from 'node:crypto'

test('anonymous /app redirects to /login', async ({ page }) => {
  await page.goto('/app')
  await expect(page).toHaveURL(/\/login/)
  await expect(page.getByRole('heading', { name: 'Sign in', exact: true })).toBeVisible()
})

test('a rejected Origin cannot create a publisher with a valid browser session, while the same trusted write succeeds', async ({ page, baseURL }) => {
  test.setTimeout(60_000)
  const origin = new URL(baseURL!).origin
  const suffix = randomUUID().replaceAll('-', '')
  const email = `admin-e2e-${suffix}@example.test`
  const password = `Disposable-only-${randomUUID()}!`
  const namespace = `e2e-${suffix}`
  const signedUp = await page.request.post('/api/auth/sign-up/email', {
    headers: { Origin: origin },
    data: { name: 'Disposable browser fixture', email, password },
  })
  await expect(signedUp).toBeOK()
  await page.context().clearCookies()

  // Authenticate through the actual built Vue form and Better Auth service.
  // No seeded token, privileged account, mocked response or CSRF exemption.
  await page.goto('/login')
  await page.getByLabel('Username or email', { exact: true }).fill(email)
  await page.getByLabel('Password', { exact: true }).fill(password)
  const signedInResponse = page.waitForResponse((response) =>
    new URL(response.url()).pathname === '/api/auth/sign-in/email' && response.request().method() === 'POST',
  { timeout: 30_000 })
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  expect((await signedInResponse).status()).toBe(200)
  await expect(page).toHaveURL(/\/app(?:\/|$)/)
  const identity = await page.request.get('/api/v1/me')
  expect(identity.status()).toBe(200)
  expect((await identity.json() as { email: string }).email).toBe(email)
  await page.goto('/app/portal')
  await expect(page.getByRole('heading', { name: 'My publishers', exact: true })).toBeVisible()
  const before = await page.request.get('/api/v1/publishers')
  expect(before.status()).toBe(200)
  expect(await before.json()).toEqual([])

  const input = page.getByLabel('Namespace', { exact: true })
  const create = page.getByRole('button', { name: 'Request namespace', exact: true })
  await input.fill(namespace)
  // page.request shares this real browser context's cookie jar. Its HTTP
  // request can carry an explicit rejected Origin, which Chromium otherwise
  // recomputes for browser fetches. No response or authentication is mocked.
  const rejected = await page.request.post('/api/v1/publishers', {
    headers: { Origin: 'https://rejected-origin.example.test' },
    data: { namespace },
  })
  expect(rejected.status()).toBe(403)
  expect((await page.request.get('/api/v1/me')).status()).toBe(200)
  const unchanged = await page.request.get('/api/v1/publishers')
  expect(unchanged.status()).toBe(200)
  expect(await unchanged.json()).toEqual([])
  await expect(input).toHaveValue(namespace)

  // Positive control: identical authenticated write with the browser's real,
  // trusted Origin must create exactly the pending namespace shown by the UI.
  const acceptedResponse = page.waitForResponse((response) =>
    new URL(response.url()).pathname === '/api/v1/publishers' && response.request().method() === 'POST')
  await create.click()
  const accepted = await acceptedResponse
  expect(accepted.status()).toBe(201)
  expect(accepted.request().postDataJSON()).toEqual({ namespace })
  await expect(page.getByRole('heading', { name: namespace, exact: true })).toBeVisible()
  const after = await page.request.get('/api/v1/publishers')
  expect(after.status()).toBe(200)
  expect(await after.json()).toEqual([expect.objectContaining({ namespace, status: 'pending', role: 'owner' })])
})
