import assert from 'node:assert/strict'
import { chromium } from 'playwright'
import { mkdir } from 'node:fs/promises'
const base = process.env.DESKTOP_URL || 'http://127.0.0.1:5174'
const out = process.env.CAPTURE_DIR
if (out) await mkdir(out, {recursive:true})
const browser = await chromium.launch({executablePath:process.env.CHROMIUM_PATH || '/usr/bin/chromium',args:['--no-sandbox']})
const url=base+'/desktop/login?'+new URLSearchParams({redirectUri:'http://127.0.0.1:9999/callback',codeChallenge:'a'.repeat(43),state:'b'.repeat(32)})
const errors=[]
async function fixture({signed=false,slowMe=false,slowLogin=false,slowHandoff=false,validCallback=false,locale='en',width=1440,dark=false}={}) {
 const context=await browser.newContext({viewport:{width,height:1000},locale,colorScheme:dark?'dark':'light',reducedMotion:'reduce'})
 const page=await context.newPage();page.on('pageerror',e=>errors.push(String(e)))
 let release;const gate=new Promise(r=>{release=r})
 await page.route('**/api/**',async route=>{
  const path=new URL(route.request().url()).pathname
  if(path==='/api/v1/me') {if(slowMe)await gate;return route.fulfill({status:signed?200:401,json:signed?{email:'demo@example.test'}:{error:'unauthorized'}})}
  if(path==='/api/v1/auth/providers')return route.fulfill({json:{google:false}})
  if(path==='/api/auth/sign-in/email') {if(slowLogin)await gate;const data=route.request().postDataJSON();signed=data.password==='demo-only';return route.fulfill({status:signed?200:401,json:{}})}
  if(path==='/api/v1/desktop/handoff'){if(slowHandoff)await gate;return route.fulfill({json:{callbackUrl:validCallback?'http://127.0.0.1:9999/callback?code=test-code&state='+ 'b'.repeat(32):'https://untrusted.test/callback?code=bad&state=bad'}})}
  return route.fulfill({status:501,json:{error:'test_unsupported'}})
 })
 await page.goto(url);return {page,context,release}
}
async function shot(page,name){await page.waitForTimeout(350);if(out)await page.screenshot({path:out+'/'+name+'.png',fullPage:true})}
try {
 const {page,context}=await fixture({dark:true})
 await page.getByLabel('Username or email',{exact:true}).fill('demo@example.test')
 await page.getByRole('button',{name:'Continue',exact:true}).click()
 await page.getByLabel('Password',{exact:true}).waitFor({state:'visible'})
 assert.equal(await page.locator('#password').evaluate(e=>e===document.activeElement),true)
 await shot(page,'password-dark')
 await page.getByLabel('Password',{exact:true}).fill('wrong')
 await page.getByRole('button',{name:'Sign in',exact:true}).last().click()
 await page.getByText('Sign-in failed.',{exact:false}).waitFor()
 await shot(page,'login-error')
 await page.getByRole('button',{name:'Back',exact:true}).click()
 assert.equal(await page.getByLabel('Username or email',{exact:true}).inputValue(),'demo@example.test')
 assert.equal(await page.locator('#password').inputValue(),'')
 await page.getByRole('button',{name:'Continue',exact:true}).click()
 await page.getByLabel('Password',{exact:true}).fill('demo-only')
 await page.getByRole('button',{name:'Sign in',exact:true}).last().click()
 await page.getByRole('button',{name:'Continue to TikTools',exact:true}).waitFor()
 assert.equal(await page.getByText('demo@example.test',{exact:true}).count(),1)
 await shot(page,'consent-dark')
 await page.getByRole('button',{name:'Continue to TikTools',exact:true}).click()
 await page.getByText('Could not complete sign-in.',{exact:false}).waitFor()
 assert.equal(new URL(page.url()).origin,base)
 await page.getByRole('button',{name:'Cancel',exact:true}).click()
 await page.getByRole('heading',{name:'Sign-in cancelled',exact:true}).waitFor()
 assert.equal(await page.locator('form,input,#continue,#cancel,#account').count(),0)
 assert.equal(await page.locator('#access-heading').evaluate(e=>e===document.activeElement),true)
 await shot(page,'cancelled-dark');await context.close()
 for(const delayed of ['slowMe','slowLogin','slowHandoff']) {
  const f=await fixture({[delayed]:true,signed:delayed!=='slowLogin'})
  if(delayed==='slowLogin'){
   await f.page.getByLabel('Username or email',{exact:true}).fill('demo@example.test')
   await f.page.getByRole('button',{name:'Continue',exact:true}).click()
   await f.page.getByLabel('Password',{exact:true}).fill('demo-only')
   await f.page.getByRole('button',{name:'Sign in',exact:true}).last().click()
  }
  if(delayed==='slowHandoff')await f.page.getByRole('button',{name:'Continue to TikTools',exact:true}).click()
  await f.page.getByRole('button',{name:'Cancel',exact:true}).click();f.release()
  await f.page.waitForTimeout(350)
  assert.equal(await f.page.locator('form,input,#continue,#account').count(),0)
  await f.context.close()
 }
 for(const locale of ['en','es','fr','de','pt']) {
  const f=await fixture({locale,width:390})
  await f.page.locator('#identifier').waitFor({state:'visible'})
  await f.page.locator('#mode-tabs button').nth(1).click()
  await f.page.locator('#display-name').waitFor({state:'visible'})
  assert.equal(await f.page.locator('#password').getAttribute('autocomplete'),'new-password')
  assert.equal(await f.page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false)
  if(locale==='es')await shot(f.page,'register-mobile-es')
  await f.page.locator('#cancel').click()
  assert.equal(await f.page.locator('form,input').count(),0)
  await f.context.close()
 }
 const valid=await fixture({signed:true,validCallback:true});await valid.page.route('http://127.0.0.1:9999/**',r=>r.fulfill({body:'Callback accepted'}));await valid.page.getByRole('button',{name:'Continue to TikTools',exact:true}).click();await valid.page.waitForURL('http://127.0.0.1:9999/callback?**');assert.equal(new URL(valid.page.url()).searchParams.get('code'),'test-code');await valid.context.close()
 const invalid=await fixture();await invalid.page.evaluate(()=>sessionStorage.clear());await invalid.page.goto(base+'/desktop/login');await invalid.page.getByText('Open TikTools and start sign-in again.').waitFor();assert.equal(await invalid.page.locator('form,#continue').count(),0);await invalid.context.close()
 assert.deepEqual(errors,[])
 console.log('PASS: login error/success, Back/focus, unique identity, valid/invalid callbacks, cancellation during session/login/handoff, invalid handoff, registration and five mobile locales; no JS errors.')
} finally {await browser.close()}
