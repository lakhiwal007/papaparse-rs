#!/usr/bin/env node
// Assembles the npm packages of a release from prebuilt addons:
//
//   papaparse-node                     the JS API and a loader (crates/papaparse-node/js/binding.cjs);
//                                      lists every platform package as an optional dependency
//   papaparse-node-<os>-<cpu>[-<abi>]  one prebuilt addon each; npm installs only the one whose
//                                      `os` / `cpu` / `libc` match the machine
//
// This is the layout NAPI-RS projects publish, without needing @napi-rs/cli.
//
// Usage: node scripts/npm-packages.mjs <binaries-dir> <out-dir>
//   <binaries-dir> holds `papaparse.<rust-target>.node` files (the release workflow's artifacts).
//   Then publish every directory in <out-dir>, platform packages first (see release.yml).

import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Rust target → npm platform fields. Keep in sync with release.yml and binding.cjs. */
export const TARGETS = {
	'aarch64-apple-darwin': { os: 'darwin', cpu: 'arm64' },
	'x86_64-apple-darwin': { os: 'darwin', cpu: 'x64' },
	'x86_64-unknown-linux-gnu': { os: 'linux', cpu: 'x64', libc: 'glibc', abi: 'gnu' },
	'aarch64-unknown-linux-gnu': { os: 'linux', cpu: 'arm64', libc: 'glibc', abi: 'gnu' },
	'x86_64-unknown-linux-musl': { os: 'linux', cpu: 'x64', libc: 'musl', abi: 'musl' },
	'aarch64-unknown-linux-musl': { os: 'linux', cpu: 'arm64', libc: 'musl', abi: 'musl' },
	'x86_64-pc-windows-msvc': { os: 'win32', cpu: 'x64', abi: 'msvc' },
	'aarch64-pc-windows-msvc': { os: 'win32', cpu: 'arm64', abi: 'msvc' },
};

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const [binaries, out] = process.argv.slice(2);
if (!binaries || !out) {
	console.error('usage: node scripts/npm-packages.mjs <binaries-dir> <out-dir>');
	process.exit(2);
}

const version = /^version = "(.+)"/m.exec(readFileSync(join(root, 'Cargo.toml'), 'utf8'))[1];
const repository = /^repository = "(.+)"/m.exec(readFileSync(join(root, 'Cargo.toml'), 'utf8'))[1];
const common = {
	version,
	license: 'MIT',
	repository: { type: 'git', url: `git+${repository}.git` },
	engines: { node: '>=18' },
};
const write = (dir, pkg) => writeFileSync(join(dir, 'package.json'), JSON.stringify(pkg, null, 2) + '\n');

rmSync(out, { recursive: true, force: true });
const platforms = {};
for (const file of readdirSync(binaries)) {
	const target = /^papaparse\.(.+)\.node$/.exec(file)?.[1];
	if (!target) continue;
	const t = TARGETS[target];
	if (!t) throw new Error(`unknown target ${target}; add it to TARGETS`);
	const name = `papaparse-node-${t.os}-${t.cpu}${t.abi ? '-' + t.abi : ''}`;
	const dir = join(out, name);
	mkdirSync(dir, { recursive: true });
	copyFileSync(join(binaries, file), join(dir, 'papaparse.node'));
	copyFileSync(join(root, 'LICENSE'), join(dir, 'LICENSE'));
	write(dir, {
		name,
		...common,
		description: `Prebuilt ${target} addon for papaparse-node`,
		main: 'papaparse.node',
		files: ['papaparse.node', 'LICENSE'],
		os: [t.os],
		cpu: [t.cpu],
		...(t.libc ? { libc: [t.libc] } : {}),
	});
	platforms[name] = version;
}
if (!Object.keys(platforms).length) throw new Error(`no papaparse.<target>.node files in ${binaries}`);

const main = join(out, 'papaparse-node');
mkdirSync(main, { recursive: true });
const files = ['index.cjs', 'binding.cjs', 'papa.cjs', 'decode.cjs', 'index.d.ts'];
for (const [file, from] of [
	['index.cjs', 'crates/papaparse-node/js'],
	['binding.cjs', 'crates/papaparse-node/js'],
	['papa.cjs', 'crates/papaparse-tape/js'],
	['decode.cjs', 'crates/papaparse-tape/js'],
	['index.d.ts', 'crates/papaparse-tape/js'],
	['README.md', 'crates/papaparse-node'],
	['LICENSE', '.'],
]) {
	if (!existsSync(join(root, from, file))) throw new Error(`missing ${from}/${file}`);
	copyFileSync(join(root, from, file), join(main, file));
}
write(main, {
	name: 'papaparse-node',
	...common,
	description: 'PapaParse-compatible CSV parser: native Node.js addon built from Rust with NAPI-RS',
	keywords: ['csv', 'parser', 'papaparse', 'napi', 'rust'],
	type: 'commonjs',
	main: 'index.cjs',
	types: 'index.d.ts',
	files: [...files, 'README.md', 'LICENSE'],
	optionalDependencies: platforms,
});
console.log(`packaged papaparse-node ${version} with ${Object.keys(platforms).join(', ')}`);
