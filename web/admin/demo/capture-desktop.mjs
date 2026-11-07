import { mkdir } from 'node:fs/promises';
import { chromium } from 'playwright';
const probe = await fetch('http://127.0.0.1:5174/api/v1/auth/providers')
if (probe.headers.get('X-Remake-Demo') !== '1') throw new Error('Start the explicit demo server before capturing; refusing actions against a non-demo origin.')
const browser=await chromium.launch({executablePath:process.env.CHROMIUM_PATH || '/usr/bin/chromium',args:['--no-sandbox']});const page=await browser.newPage({viewport:{width:1440,height:1000}}); const base='http://127.0.0.1:5174';
const url=base+'/desktop/login?'+new URLSearchParams({redirectUri:'http://127.0.0.1:9999/callback',codeChallenge:'a'.repeat(43),state:'b'.repeat(32)});
const out=new URL('../../../docs/demo/screenshots/', import.meta.url).pathname;
await mkdir(out, { recursive: true });
const errors = [];
page.on('pageerror', error => errors.push(String(error)));
await page.goto(url);await page.locator('#app > *').waitFor();await page.waitForTimeout(700);await page.screenshot({path:out+'desktop-consent.png',fullPage:true});
await page.request.post(base+'/api/auth/sign-out');await page.reload();await page.waitForTimeout(500);await page.screenshot({path:out+'desktop-login.png',fullPage:true});console.log(await page.locator('body').innerText());
await page.setViewportSize({width:390,height:844});await page.screenshot({path:out+'desktop-login-mobile.png',fullPage:true});await page.setViewportSize({width:1440,height:1000});
await page.locator('input[autocomplete="username"]').fill('demo@example.test');
await page.getByRole('button',{name:'Continue',exact:true}).click();await page.waitForTimeout(200);
await page.screenshot({path:out+'desktop-password.png',fullPage:true});
await page.getByRole('button',{name:'Create account',exact:true}).click();await page.waitForTimeout(200);
await page.screenshot({path:out+'desktop-register.png',fullPage:true});
await browser.close();
if (errors.length) throw new Error(errors.join('\n'));
