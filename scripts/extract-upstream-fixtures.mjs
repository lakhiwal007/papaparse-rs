#!/usr/bin/env node
// Extracts PapaParse's own test suites (tests/test-cases.js) into JSON fixtures
// that the Rust conformance harness consumes.
//
// Usage: node scripts/extract-upstream-fixtures.mjs <path-to-papaparse-checkout>
//
// Values JSON cannot represent are tagged so the harness can recognise them:
//   functions -> {"$fn": "<source>"}     (test needs a callback; harness skips it)
//   Date      -> {"$date": "<iso>" | "Invalid Date"}
//   NaN/±Inf  -> {"$num": "NaN" | "Infinity" | "-Infinity"}
//   undefined -> {"$undefined": true}

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const upstream = process.argv[2];
if (!upstream) {
	console.error('usage: extract-upstream-fixtures.mjs <papaparse-checkout>');
	process.exit(2);
}

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'conformance', 'fixtures');
const SUITES = ['CORE_PARSER_TESTS', 'PARSE_TESTS', 'PARSE_ASYNC_TESTS', 'UNPARSE_TESTS', 'CUSTOM_TESTS'];

const source = readFileSync(join(upstream, 'tests', 'test-cases.js'), 'utf8')
	+ `\n;globalThis.__suites = { ${SUITES.join(', ')} };`;

const noop = () => {};
noop.skip = noop;
const sandbox = {
	module: undefined,
	chai: { assert: {} },
	Papa: {},
	describe: noop,
	it: noop,
	document: { getElementById: () => ({ src: '' }) },
	console,
};
vm.createContext(sandbox);
vm.runInContext(source, sandbox, { filename: 'test-cases.js' });

function tag(value) {
	if (typeof value === 'function') return { $fn: value.toString() };
	if (value === undefined) return { $undefined: true };
	if (typeof value === 'number' && !Number.isFinite(value)) return { $num: String(value) };
	if (Object.prototype.toString.call(value) === '[object Date]') return { $date: isNaN(value) ? 'Invalid Date' : value.toISOString() };
	if (Array.isArray(value)) return value.map(tag);
	if (value && typeof value === 'object') {
		const out = {};
		for (const [k, v] of Object.entries(value)) out[k] = tag(v);
		return out;
	}
	return value;
}

mkdirSync(outDir, { recursive: true });
const pkg = JSON.parse(readFileSync(join(upstream, 'package.json'), 'utf8'));
for (const name of SUITES) {
	const cases = sandbox.__suites[name].map(tag);
	const file = join(outDir, name.toLowerCase().replace(/_tests$/, '') + '.json');
	writeFileSync(file, JSON.stringify({ upstreamVersion: pkg.version, suite: name, cases }, null, '\t') + '\n');
	console.log(`${name}: ${cases.length} cases -> ${file}`);
}
