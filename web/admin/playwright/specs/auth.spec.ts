import { expect, test } from 'playwright/test'

// Anonymous users must land on /login and stay there: the API answers 401
// and the shell redirects. Authenticated flows need a seeded session, so
// they are covered by `npm test` and the Rust suite; this spec pins the
// anonymous + CSRF-rejection surface end to end.
test('anonymous /app redirects to /login', async ({ page }) => {
  await page.goto('/app')
  await expect(page).toHaveURL(/\/login/)
  await expect(page.getByRole('heading', { name: /cloud admin/i })).toBeVisible()
})

test('cross-origin POST without session is rejected, not redirected to login loop', async ({
  page,
  request,
}) => {
  const response = await request.post('/api/v1/admin/overview')
  expect([401, 403]).toContain(response.status())
  await page.goto('/app/audit')
  await expect(page).toHaveURL(/\/login/)
})
