const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const script = fs.readFileSync('crates/cloud-api/web/desktop-login.js', 'utf8');
const valid = new URL('https://cloud.example/desktop/login');
valid.searchParams.set('redirectUri', 'http://127.0.0.1:49152/callback');
valid.searchParams.set('codeChallenge', 'a'.repeat(43));
valid.searchParams.set('state', 'state-0123456789-abcdef');
const tick = () => new Promise((resolve) => setImmediate(resolve));
function setup(href, responses, saved = new Map()) {
  const elements = {};
  for (const id of ['message', 'account', 'sign-in', 'continue', 'cancel']) {
    elements[id] = { hidden: true, disabled: false, textContent: '', listeners: {}, addEventListener(event, cb) { this.listeners[event] = cb; }, click() { return this.listeners.click?.(); } };
  }
  const url = new URL(href);
  const location = { pathname: url.pathname, search: url.search, assigned: null, assign(target) { this.assigned = target; } };
  const storage = { setItem(k, v) { saved.set(k, v); }, getItem(k) { return saved.get(k) ?? null; }, removeItem(k) { saved.delete(k); } };
  const calls = [];
  const fetch = (path, options) => { calls.push({ path, options }); return responses(path, options); };
  const history = { replaceState(_a, _b, path) { location.pathname = path; location.search = ''; } };
  vm.runInNewContext(script, { document: { getElementById: (id) => elements[id] }, location, history, sessionStorage: storage, fetch, URL, URLSearchParams });
  return { elements, location, saved, calls };
}
const response = (status, body) => ({ status, ok: status >= 200 && status < 300, json: async () => body });

test('requires explicit consent and navigates only to the requested loopback', async () => {
  const env = setup(valid.href, async (path) => path === '/api/v1/me' ? response(200, { email: '<img src=x>' }) : response(200, { callbackUrl: 'http://127.0.0.1:49152/callback?code=one-time&state=state-0123456789-abcdef' }));
  await tick();
  assert.equal(env.elements.account.textContent, '<img src=x>');
  assert.equal(env.elements.continue.hidden, false);
  assert.equal(env.calls.length, 1);
  assert.equal(env.location.search, '');
  await env.elements.continue.click();
  assert.equal(env.calls[1].path, '/api/v1/desktop/handoff');
  assert.equal(env.location.assigned, 'http://127.0.0.1:49152/callback?code=one-time&state=state-0123456789-abcdef');
  assert.equal(env.saved.size, 0);
});
test('retains handoff through login and resumes after OAuth callback', async () => {
  const env = setup(valid.href, async () => response(401, {}));
  await tick();
  assert.equal(env.elements['sign-in'].hidden, false);
  env.elements['sign-in'].click();
  assert.equal(env.location.assigned, '/api/auth/sign-in/social/google');
  const resumed = setup('https://cloud.example/', async () => response(200, { email: 'person@example.test' }), env.saved);
  await tick();
  assert.equal(resumed.elements.continue.hidden, false);
  assert.equal(resumed.elements.account.textContent, 'person@example.test');
});
test('rejects invalid input and an unexpected callback target', async () => {
  const bad = new URL(valid); bad.searchParams.append('state', 'extra');
  const rejected = setup(bad.href, async () => { throw new Error('unexpected request'); });
  assert.equal(rejected.calls.length, 0);
  assert.equal(rejected.elements.continue.hidden, true);
  const env = setup(valid.href, async (path) => path === '/api/v1/me' ? response(200, { email: 'a' }) : response(200, { callbackUrl: 'https://evil.example/callback?code=x&state=state-0123456789-abcdef' }));
  await tick(); await env.elements.continue.click();
  assert.equal(env.location.assigned, null);
  assert.equal(env.elements.continue.hidden, true);
});
test('cancelling while the session check is pending cannot resume consent', async () => {
  let resolve;
  const env = setup(valid.href, () => new Promise((r) => { resolve = r; }));
  env.elements.cancel.click(); resolve(response(200, { email: 'a' })); await tick();
  assert.equal(env.elements.continue.hidden, true);
  assert.equal(env.location.assigned, null);
  assert.equal(env.saved.size, 0);
});
test('cancelling while handoff is in flight prevents navigation', async () => {
  let finish;
  const env = setup(valid.href, (path) => path === '/api/v1/me' ? Promise.resolve(response(200, { email: 'a' })) : new Promise((resolve) => { finish = resolve; }));
  await tick();
  const pending = env.elements.continue.click();
  env.elements.cancel.click();
  finish(response(200, { callbackUrl: 'http://127.0.0.1:49152/callback?code=one-time&state=state-0123456789-abcdef' }));
  await pending;
  assert.equal(env.location.assigned, null);
  assert.equal(env.saved.size, 0);
});
