#!/usr/bin/env node
// Generate a disposable test registry with TikTools' own tooling. Never use for real keys.
// Replaces scripts/build-registry-fixture.sh. Node built-ins only.
//
// Usage: build-registry-fixture.ts <TikTools PR 43 checkout> <new output directory>
import { spawnSync } from 'node:child_process';
import { copyFileSync, cpSync, existsSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const REVISION = '983a375c5b702681a791061ca863cea9c542528d';

function fail(message: string, code = 1): never {
  console.error(message);
  process.exit(code);
}

function git(repo: string, args: string[]): string {
  const result = spawnSync('git', ['-C', repo, ...args], { encoding: 'utf8' });
  if (result.status !== 0) fail(`git ${args.join(' ')} failed in ${repo}`);
  return result.stdout.trim();
}

const [checkout, outputArg] = process.argv.slice(2);
if (process.argv.length - 2 !== 2 || !checkout || !outputArg) {
  fail('usage: build-registry-fixture.ts <TikTools checkout> <new output directory>', 2);
}
const repo = resolve(checkout);
const output = resolve(outputArg);
if (git(repo, ['rev-parse', 'HEAD']) !== REVISION) fail(`unexpected TikTools revision; expected ${REVISION}`);
if (spawnSync('git', ['-C', repo, 'diff', '--quiet']).status !== 0 || spawnSync('git', ['-C', repo, 'diff', '--cached', '--quiet']).status !== 0) {
  fail('TikTools checkout has tracked changes');
}
if (existsSync(output) || existsSync(`${output}.root-pin.json`)) fail('output must not already exist');

const work = mkdtempSync(join(tmpdir(), 'registry-fixture-'));
const env = { ...process.env, CARGO_TARGET_DIR: join(repo, 'target') };
try {
  process.umask(0o077);
  const cli = join(repo, 'target', 'debug', 'tiktools-plugin');
  const cargo = (args: string[]) => {
    const result = spawnSync('cargo', args, { stdio: 'inherit', env });
    if (result.status !== 0) process.exit(result.status ?? 1);
  };
  const tool = (args: string[]) => {
    const result = spawnSync(cli, args, { stdio: 'inherit', env });
    if (result.status !== 0) process.exit(result.status ?? 1);
  };

  cargo(['build', '--locked', '--manifest-path', join(repo, 'Cargo.toml'), '-p', 'tiktools-plugin-devkit', '--bin', 'tiktools-plugin']);
  tool(['new', 'acme.hooks', '--template', 'declarative', '--dir', join(work, 'plugin')]);
  tool(['keygen', '--id', 'fixture-root', '--role', 'root', '--out', join(work, 'keys')]);
  tool(['keygen', '--id', 'fixture-index', '--role', 'index', '--out', join(work, 'keys')]);
  tool(['keygen', '--id', 'fixture-acme', '--publisher', 'acme', '--out', join(work, 'keys')]);
  tool(['pack', join(work, 'plugin'), '--out-dir', join(work, 'dist'), '--sign', join(work, 'keys', 'fixture-acme.key.json')]);
  tool([
    'index', 'init', join(work, 'registry'), '--name', 'fixture',
    '--root-key', join(work, 'keys', 'fixture-root.key.json'),
    '--index-key', join(work, 'keys', 'fixture-index.key.json'),
    '--delegate', join(work, 'keys', 'fixture-acme.pub.json'),
  ]);
  tool([
    'publish', join(work, 'dist', 'acme.hooks-0.1.0.plugin'),
    '--registry', join(work, 'registry'),
    '--index-key', join(work, 'keys', 'fixture-index.key.json'),
  ]);

  mkdirSync(output, { recursive: true });
  cpSync(join(work, 'registry'), output, { recursive: true });
  copyFileSync(join(work, 'keys', 'fixture-root.pub.json'), `${output}.root-pin.json`);
  console.log(`Disposable test registry written to ${output}. Its keys are not production trust anchors.`);
} finally {
  rmSync(work, { recursive: true, force: true });
}
