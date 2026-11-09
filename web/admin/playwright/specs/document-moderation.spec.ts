import { expect, test } from 'playwright/test'
import { randomUUID } from 'node:crypto'
import { spawnSync } from 'node:child_process'

// Grants one freshly created fixture account a reviewer role. The same strict
// database boundary as the E2E launcher prevents touching an application DB.
function grantFixtureModerator(email: string) {
  const raw = process.env.CLOUD_ADMIN_E2E_DATABASE_URL
  if (!raw || !/^(?:author|reviewer)-[a-f0-9]+@example\.test$/.test(email)) throw new Error('Invalid disposable moderator fixture')
  let database: URL
  try { database = new URL(raw) } catch { throw new Error('Invalid disposable database URL') }
  if (!['postgres:', 'postgresql:'].includes(database.protocol) || !['127.0.0.1', 'localhost', '[::1]'].includes(database.hostname) || !/^\/cloud_admin_e2e(?:_[a-zA-Z0-9_-]+)?$/.test(database.pathname) || database.search || database.hash) {
    throw new Error('Refusing moderator fixture outside the disposable loopback database')
  }
  const sql = `INSERT INTO cloud_account_roles(user_id,role) SELECT id,'moderator' FROM "user" WHERE email='${email}';`
  const result = spawnSync('psql', [raw, '-X', '-v', 'ON_ERROR_STOP=1', '-c', sql], { encoding: 'utf8' })
  if (result.status !== 0 || !result.stdout.includes('INSERT 0 1')) throw new Error('Could not grant the disposable moderator role')
}

test('an independent moderator reviews and withdraws an exact Cloud document through the built UI', async ({ browser, page, baseURL }) => {
  test.setTimeout(60_000)
  const origin = new URL(baseURL!).origin
  const suffix = randomUUID().replaceAll('-', '')
  const authorEmail = `author-${suffix}@example.test`
  const moderatorEmail = `reviewer-${suffix}@example.test`
  const password = `Disposable-only-${randomUUID()}!`
  const author = await browser.newContext({ baseURL })
  try {
    const authorSignup = await author.request.post('/api/auth/sign-up/email', { headers: { Origin: origin }, data: { name: 'Disposable author', email: authorEmail, password } })
    await expect(authorSignup).toBeOK()
    const document = { templateVersion: 1, id: 'gift-rule', title: `Gift rule ${suffix}`, actions: [{ name: 'Notify', typeId: 'core.notify', enabled: true, config: { message: 'Thanks {{ event.user.nickname }}' } }], event: { name: 'Gift', trigger: 'tiktok.gift' } }
    const created = await author.request.post('/api/v2/library/documents', { headers: { Origin: origin }, data: { kind: 'template', document } })
    expect(created.status()).toBe(201)
    const { id } = await created.json() as { id: string }
    const stored = await author.request.get(`/api/v2/library/documents/${id}`)
    await expect(stored).toBeOK()
    const { sha256 } = await stored.json() as { sha256: string }
    expect((await author.request.post(`/api/v2/library/documents/${id}/versions/1/submit`, { headers: { Origin: origin } })).status()).toBe(204)

    const signup = await page.request.post('/api/auth/sign-up/email', { headers: { Origin: origin }, data: { name: 'Disposable reviewer', email: moderatorEmail, password } })
    await expect(signup).toBeOK()
    grantFixtureModerator(authorEmail)
    grantFixtureModerator(moderatorEmail)
    const ownQueue = await author.request.get('/api/v2/moderation/documents')
    expect(ownQueue.status()).toBe(200)
    expect((await ownQueue.json() as { items: { id: string }[] }).items.some((item) => item.id === id)).toBe(false)
    const ownDecision = await author.request.post(`/api/v2/moderation/documents/${id}/versions/1/decision`, { headers: { Origin: origin }, data: { decision: 'approve', reason: 'Must be refused' } })
    expect(ownDecision.status()).toBe(403)
    await page.context().clearCookies()
    await page.goto('/login')
    await page.getByLabel('Username or email', { exact: true }).fill(moderatorEmail)
    await page.getByLabel('Password', { exact: true }).fill(password)
    await page.getByRole('button', { name: 'Sign in', exact: true }).click()
    await expect(page).toHaveURL(/\/app(?:\/|$)/)
    await page.goto('/app/moderation/documents')
    await page.getByRole('button', { name: new RegExp(`Gift rule ${suffix}`) }).click()
    await expect(page.getByTestId('document-content')).toContainText(document.actions[0]!.config.message)
    await expect(page.getByText(sha256)).toBeVisible()
    await page.getByRole('button', { name: 'Approve', exact: true }).click()
    const approval = page.getByRole('dialog')
    await expect(approval).toContainText(sha256)
    await approval.getByLabel(/Required reason/).fill('Reviewed actions and parameters')
    await approval.getByRole('button', { name: 'Confirm', exact: true }).click()
    await expect(page.getByText('Decision saved. The list has been refreshed.')).toBeVisible()
    const publicDocument = await page.request.get(`/api/v2/catalog/documents/${id}/versions/1`)
    expect(publicDocument.status()).toBe(200)
    expect((await publicDocument.json() as { document: unknown }).document).toEqual(document)

    await page.getByRole('button', { name: 'Published', exact: true }).click()
    await page.getByRole('button', { name: new RegExp(`Gift rule ${suffix}`) }).click()
    await expect(page.getByTestId('document-content')).toContainText(document.actions[0]!.config.message)
    await page.getByRole('button', { name: 'Withdraw publication', exact: true }).click()
    const withdrawal = page.getByRole('dialog')
    await expect(withdrawal).toContainText(sha256)
    await withdrawal.getByLabel(/Required reason/).fill('Outdated instructions')
    await withdrawal.getByRole('button', { name: 'Confirm', exact: true }).click()
    await expect(page.getByText('Decision saved. The list has been refreshed.')).toBeVisible()
    await expect.poll(async () => (await page.request.get(`/api/v2/catalog/documents/${id}/versions/1`)).status()).toBe(404)
  } finally {
    await author.close()
  }
})
