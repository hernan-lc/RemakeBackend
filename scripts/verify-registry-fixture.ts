#!/usr/bin/env node
// Verify a disposable registry fixture with TikTools' pinned registry client.
// Replaces scripts/verify-registry-fixture.sh. Node built-ins only.
//
// Usage: verify-registry-fixture.ts <TikTools checkout> <fixture directory> <root pin file>
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const REVISION = '983a375c5b702681a791061ca863cea9c542528d';

function fail(message: string, code = 1): never {
  console.error(message);
  process.exit(code);
}

const args = process.argv.slice(2);
if (args.length !== 3) fail('usage: verify-registry-fixture.ts <TikTools checkout> <fixture directory> <root pin file>', 2);
const [checkout, fixtureArg, pinArg] = args as [string, string, string];
const repo = resolve(checkout);
const fixture = resolve(fixtureArg);
const pin = resolve(pinArg);

const git = (gitArgs: string[]) => spawnSync('git', ['-C', repo, ...gitArgs], { encoding: 'utf8' });
const head = git(['rev-parse', 'HEAD']);
if (head.status !== 0 || head.stdout.trim() !== REVISION) fail('unexpected TikTools revision');
if (git(['diff', '--quiet']).status !== 0 || git(['diff', '--cached', '--quiet']).status !== 0) {
  fail('TikTools checkout has tracked changes');
}

const scriptDir = dirname(fileURLToPath(import.meta.url));
const work = mkdtempSync(join(tmpdir(), 'registry-verify-'));
try {
  mkdirSync(join(work, 'src'), { recursive: true });
  mkdirSync(join(work, 'scratch'), { recursive: true });
  copyFileSync(join(scriptDir, '..', 'tests', 'registry-fixture-client', 'main.rs'), join(work, 'src', 'main.rs'));
  copyFileSync(join(repo, 'Cargo.lock'), join(work, 'Cargo.lock'));
  // TOML basic strings: escape backslashes and double quotes in the checkout path.
  const escaped = repo.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
  writeFileSync(join(work, 'Cargo.toml'), `[package]
name = "cloud-registry-fixture-verifier"
version = "0.1.0"
edition = "2021"
[dependencies]
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "io-util"] }
tiktools-plugin-package = { path = "${escaped}/crates/tiktools-plugin-package" }
tiktools-plugin-registry = { path = "${escaped}/crates/tiktools-plugin-registry" }
`);
  const result = spawnSync('cargo', [
    'run', '--offline', '--manifest-path', join(work, 'Cargo.toml'), '--', fixture, pin, join(work, 'scratch'),
  ], { stdio: 'inherit', env: { ...process.env, CARGO_TARGET_DIR: join(repo, 'target') } });
  if (result.status !== 0) process.exit(result.status ?? 1);
} finally {
  rmSync(work, { recursive: true, force: true });
}
