const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { stripTypeScriptTypes } = require('node:module');
// Run the current Vue controller and its real API/handoff modules with small
// Vue/DOM stand-ins. Rendering is outside this transport/consent test.
const moduleScript = (path) => stripTypeScriptTypes(fs.readFileSync(path, 'utf8'))
  .replace(/^export /gm, '');
const component = fs.readFileSync('web/desktop/src/App.vue', 'utf8');
const controller = component.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1];
const script = [
  moduleScript('web/desktop/src/handoff.ts'),
  moduleScript('web/desktop/src/api.ts'),
  stripTypeScriptTypes(controller.replace(/^import .*;\n/gm, '')),
  'globalThis.controller = { account, canContinue, cancelled, pending, googleAvailable, cont, cancel, signInWithGoogle };',
].join('\n');
const valid = new URL('https://cloud.example/desktop/login');
valid.searchParams.set('redirectUri', 'http://127.0.0.1:49152/callback');
valid.searchParams.set('codeChallenge', 'a'.repeat(43));
valid.searchParams.set('state', 'state-0123456789-abcdef');
const tick = () => new Promise((resolve) => setImmediate(resolve));
function setup(href, responses, saved = new Map()) {
  const url = new URL(href);
  const location = { pathname: url.pathname, search: url.search, assigned: null, assign(target) { this.assigned = target; } };
  const storage = { setItem(k, v) { saved.set(k, v); }, getItem(k) { return saved.get(k) ?? null; }, removeItem(k) { saved.delete(k); } };
  const calls = [];
  const fetch = (path, options) => { calls.push({ path, options }); return responses(path, options); };
  const history = { replaceState(_a, _b, path) { location.pathname = path; location.search = ''; } };
  let mounted;
  const context = {
    document: { documentElement: {}, getElementById: () => null },
    location, history, sessionStorage: storage, fetch, URL, URLSearchParams,
    ref: (value = null) => ({ value }),
    nextTick: async (fn) => fn?.(),
    watchEffect: (fn) => fn(),
    onMounted: (fn) => { mounted = fn; },
    STRINGS: { en: new Proxy({}, { get: (_target, key) => key }) },
    detectLanguage: () => 'en', persistLanguage: () => {},
  };
  vm.runInNewContext(script, context);
  const state = context.controller;
  const elements = {
    account: { get textContent() { return state.account.value; } },
    continue: { get hidden() { return !state.canContinue.value; }, click: () => state.cont() },
    'sign-in': { get hidden() { return !state.pending.value || state.cancelled.value || state.canContinue.value; }, click: () => state.signInWithGoogle() },
    cancel: { click: () => state.cancel() },
  };
  void mounted();
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
  const env = setup(valid.href, async (path) => path === '/api/v1/auth/providers' ? response(200, { google: true }) : response(401, {}));
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
