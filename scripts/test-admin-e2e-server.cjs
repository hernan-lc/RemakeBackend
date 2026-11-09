const { test } = require('node:test')
const assert = require('node:assert/strict')
const { spawnSync } = require('node:child_process')
const path = require('node:path')

const launcher = path.join(__dirname, 'admin-e2e-server.mjs')
const fixtureDatabase = 'postgres://fixture@127.0.0.1:5432/cloud_admin_e2e'
const privateMarker = 'fixture-password-must-not-be-logged'
const cases = [
  ['missing explicit database', {}, /Set CLOUD_ADMIN_E2E_DATABASE_URL/],
  ['remote database', { CLOUD_ADMIN_E2E_DATABASE_URL: `postgres://fixture:${privateMarker}@database.example.test/cloud_admin_e2e` }, /refuses databases/],
  ['application database on loopback', { CLOUD_ADMIN_E2E_DATABASE_URL: 'postgres://fixture@127.0.0.1:5432/cloud' }, /refuses databases/],
  ['non-PostgreSQL database', { CLOUD_ADMIN_E2E_DATABASE_URL: 'mysql://fixture@127.0.0.1:5432/cloud_admin_e2e' }, /refuses databases/],
  ['malformed credential-bearing URL', { CLOUD_ADMIN_E2E_DATABASE_URL: `postgres://fixture:${privateMarker}@[invalid/cloud_admin_e2e` }, /valid disposable loopback database/],
  ['public application origin', { CLOUD_ADMIN_E2E_DATABASE_URL: fixtureDatabase, ADMIN_URL: 'https://cloud.example.test' }, /plain 127.0.0.1 origin/],
  ['credential-bearing application origin', { CLOUD_ADMIN_E2E_DATABASE_URL: fixtureDatabase, ADMIN_URL: `http://fixture:${privateMarker}@127.0.0.1:5183` }, /plain 127.0.0.1 origin/],
  ['malformed application origin', { CLOUD_ADMIN_E2E_DATABASE_URL: fixtureDatabase, ADMIN_URL: privateMarker }, /valid loopback origin/],
]

for (const [name, env, message] of cases) {
  test(`fixture refuses ${name} before launching a service`, () => {
    const result = spawnSync(process.execPath, [launcher], {
      env: { PATH: process.env.PATH, ...env },
      encoding: 'utf8',
      timeout: 5_000,
    })
    assert.equal(result.error, undefined)
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, message)
    assert.equal(`${result.stdout}${result.stderr}`.includes(privateMarker), false)
  })
}
