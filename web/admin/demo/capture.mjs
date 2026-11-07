import assert from 'node:assert/strict'
import { chromium } from 'playwright'
import { mkdir, writeFile } from 'node:fs/promises'
const output = new URL('../../../docs/demo/screenshots/', import.meta.url).pathname
await mkdir(output, { recursive: true })
const probe = await fetch('http://127.0.0.1:5173/api/v1/auth/providers')
if (probe.headers.get('X-Remake-Demo') !== '1') throw new Error('Start the explicit demo server before capturing; refusing actions against a non-demo origin.')
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || '/usr/bin/chromium', args: ['--no-sandbox'] })
const errors = [], captures = []
const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'reduce' })
const page = await context.newPage()
page.on('pageerror', e => errors.push(String(e)))
const origin = 'http://127.0.0.1:5173'
async function capture(name, path) {
  if (path) await page.goto(origin + path)
  await page.locator('#app > *').waitFor(); await page.waitForTimeout(500)
  await page.screenshot({ path: output + name + '.png', fullPage: true })
  captures.push({ name, url: page.url(), overflow: await page.evaluate(() => document.documentElement.scrollWidth > innerWidth) })
}
for (const [name, path] of Object.entries({ dashboard:'/app', publishers:'/app/publishers', moderation:'/app/moderation', audit:'/app/audit', portal:'/app/portal', account:'/app/account', login:'/login', forbidden:'/403', callback:'/auth/callback?error=demo' })) await capture(name, path)
await page.goto(origin + '/app/portal'); await page.locator('#app > *').waitFor(); await page.waitForTimeout(500)
for (const label of ['Members', 'Signing keys', 'Ownership transfer', 'Releases']) {
  const button = page.getByRole('button', {name:label, exact:true}).first()
  if (await button.count()) { await button.click(); await capture('portal-' + label.toLowerCase().replaceAll(' ','-')) }
  else throw new Error('Missing panel button: '+label)
}
await page.goto(origin + '/app/portal'); await page.getByRole('button', {name:'Signing keys',exact:true}).click()
await page.getByRole('button', {name:'Enroll signing key',exact:true}).click(); await capture('step-up-dialog')
await page.goto(origin + '/app/moderation'); await page.locator('summary').filter({hasText:'File details'}).first().click(); await capture('moderation-details')
await page.goto(origin + '/app/publishers'); await page.locator('#app > *').waitFor(); await page.waitForTimeout(500)
await page.getByRole('button',{name:'Approve',exact:true}).first().click(); await capture('publisher-confirmation')
await page.goto(origin + '/app')
await page.evaluate(() => localStorage.setItem('tiktools-admin-theme','dark'))
for (const [name,path] of Object.entries({dashboard:'/app',moderation:'/app/moderation',portal:'/app/portal'})) await capture(name+'-dark',path)
await page.evaluate(() => localStorage.setItem('tiktools-admin-theme','light'))
await page.setViewportSize({width:390,height:844})
for (const [name,path] of Object.entries({dashboard:'/app',publishers:'/app/publishers',moderation:'/app/moderation',audit:'/app/audit',portal:'/app/portal',account:'/app/account',login:'/login'})) await capture(name+'-mobile',path)
// Exercise actual UI mutations after baseline captures.
await page.goto(origin + '/app/portal')
const namespace = 'demo-browser-' + Date.now()
await page.getByLabel('Namespace', {exact:true}).fill(namespace)
await page.getByRole('button', {name:'Request namespace',exact:true}).click()
await page.getByRole('heading', {name:namespace,exact:true}).waitFor()
await page.goto(origin + '/logout')
await page.waitForURL('**/login*')
await page.getByLabel('Username or email', {exact:true}).fill('demo@example.test')
await page.getByLabel('Password', {exact:true}).fill('demo-only')
await page.getByRole('button', {name:'Sign in',exact:true}).click()
await page.waitForURL('**/app')
await page.getByRole('heading',{name:'Dashboard',exact:true}).waitFor()
assert.equal(await page.getByRole('heading',{name:'Dashboard',exact:true}).count(),1)
await writeFile(output+'report.json',JSON.stringify({errors,captures},null,2))
console.log(JSON.stringify({errors,captures},null,2))
await browser.close()
if(errors.length || captures.some(c=>c.overflow)) process.exitCode=1
