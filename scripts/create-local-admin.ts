#!/usr/bin/env node
// Create a disposable administrator in the default local development database.
// Replaces scripts/create-local-admin.sh. Node built-ins only; uses the `psql` client
// to read the new account id, exactly as the shell version did.
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { randomBytes } from 'node:crypto';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
process.chdir(root);

const databaseUrl = 'postgres://cloud:local-only-password@127.0.0.1:5432/cloud';
const appOrigin = 'http://127.0.0.1:5173';
process.env.DATABASE_URL = databaseUrl;
process.env.APP_ORIGIN = appOrigin;

function fail(message: string): never {
  console.error(message);
  process.exit(1);
}

if (!process.env.AUTH_SECRET) {
  if (!existsSync('.local-dev/auth-secret') || readFileSync('.local-dev/auth-secret', 'utf8').length === 0) {
    fail('Start `cargo run -p devtools -- dev --ui` first to initialize the local secret and services.');
  }
  process.env.AUTH_SECRET = readFileSync('.local-dev/auth-secret', 'utf8');
}

process.umask(0o077);
mkdirSync('.local-dev', { recursive: true });
const suffix = randomBytes(8).toString('hex');
const email = `admin-${suffix}@example.test`;
const password = randomBytes(24).toString('base64url');
const credentials = `.local-dev/test-admin-${suffix}.txt`;
writeFileSync(credentials, `Email: ${email}\nPassword: ${password}\n`);

const signup = await fetch(`${appOrigin}/api/auth/sign-up/email`, {
  method: 'POST',
  headers: { Origin: appOrigin, 'Content-Type': 'application/json' },
  body: JSON.stringify({ name: 'Local Test Admin', email, password }),
});
if (!signup.ok) {
  fail('Sign-up failed. Ensure `remake-dev dev --ui` is running with its default local database.');
}
await signup.arrayBuffer();

// Look up the generated email in the intended local database before granting a role.
const lookup = spawnSync('psql', ['-X', '-At', '-v', 'ON_ERROR_STOP=1', databaseUrl, '-c',
  `SELECT id FROM "user" WHERE email = '${email}'`], { encoding: 'utf8' });
if (lookup.status !== 0) fail(lookup.stderr.trim() || 'psql failed');
const userId = lookup.stdout.trim();
if (!userId || userId.includes('\n')) {
  fail('The new account was not found in the default local database; no role was granted.');
}

const grant = spawnSync('cargo', ['run', '-p', 'cloud-api', '--', 'grant-role', userId, 'admin', 'local-dev-script'], { stdio: 'inherit' });
if (grant.status !== 0) process.exit(grant.status ?? 1);

console.log(`Local test administrator created. Credentials saved in ${credentials}:`);
console.log(readFileSync(credentials, 'utf8').trimEnd());
