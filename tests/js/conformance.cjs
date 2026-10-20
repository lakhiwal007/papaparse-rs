'use strict';
// Runs upstream PapaParse fixtures against a built package (wasm or Node-API) and compares
// real JS values (including Date objects) the way PapaParse's own mocha suite does.
// Callbacks in the fixtures (`{"$fn": source}`) are revived as real functions, and the
// `custom` suite's test bodies run against this package as `Papa`.
// Usage: node conformance.cjs <pkg-dir>
const assert = require('node:assert/strict');
const path = require('node:path');
const Papa = require(path.resolve(process.argv[2], 'index.cjs'));
const fixtures = path.join(__dirname, '../../conformance/fixtures');

/** Converts fixture tags back to JS values. */
function revive(v) {
	if (Array.isArray(v)) return v.map(revive);
	if (v && typeof v === 'object') {
		if ('$date' in v) return new Date(v.$date === 'Invalid Date' ? NaN : v.$date);
		if ('$num' in v) return Number(v.$num);
		if ('$fn' in v) return new Function('Papa', `return (${v.$fn});`)(Papa);
		if ('$undefined' in v) return undefined;
		return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, revive(x)]));
	}
	return v;
}

let pass = 0, fail = 0, skip = 0;
function report(file, c, e) {
	fail++;
	console.log(`FAIL ${file}: ${c.description}\n  ${String(e && e.message).split('\n').slice(0, 12).join('\n  ')}`);
}

function runParseSuites() {
	for (const file of ['core_parser.json', 'parse.json']) {
		const isCore = file === 'core_parser.json';
		for (const c of require(path.join(fixtures, file)).cases) {
			// Core-parser cases use `new Papa.Parser`, which has no auto-detection; give an explicit delimiter.
			const config = revive(c.config ?? {});
			// Core cases with `header` test `new Papa.Parser`'s raw rows, which `parse` turns into objects.
			const notApplicable = isCore && (c.config?.header || typeof c.config?.comments === 'number');
			if (c.disabled || notApplicable) { skip++; continue; }
			if (isCore && !config.delimiter) config.delimiter = ',';
			try {
				const actual = Papa.parse(c.input, config);
				const expected = revive(c.expected);
				assert.deepEqual(actual.data, expected.data);
				assert.deepEqual(actual.errors, expected.errors);
				if (!isCore && expected.meta) assert.deepEqual(actual.meta, expected.meta);
				pass++;
			} catch (e) {
				report(file, c, e);
			}
		}
	}
}

function runUnparseSuite() {
	for (const c of require(path.join(fixtures, 'unparse.json')).cases) {
		if (c.disabled) { skip++; continue; }
		try {
			assert.equal(Papa.unparse(revive(c.input), revive(c.config)), c.expected);
			pass++;
		} catch (e) {
			report('unparse.json', c, e);
		}
	}
}

/** Runs a mocha-style `function(callback)` test; fails if `callback` is called twice. */
function runAsync(fn, timeout) {
	return new Promise((resolve, reject) => {
		const calls = [];
		const timer = setTimeout(() => reject(new Error('timed out')), timeout);
		fn((value) => {
			calls.push(value);
			if (calls.length === 1) {
				clearTimeout(timer);
				// Give a stray second call (e.g. `complete` after a pause) a chance to show up.
				setTimeout(() => calls.length === 1 ? resolve(value) : reject(new Error(`callback called ${calls.length} times: ${JSON.stringify(calls)}`)), 100);
			}
		});
	});
}

async function runCustomSuite() {
	for (const c of require(path.join(fixtures, 'custom.json')).cases) {
		if (c.disabled) { skip++; continue; }
		// Web workers do not exist in Node; `worker: true` parses on this thread instead.
		try {
			const actual = await runAsync(revive(c.run), c.timeout || 5000);
			assert.deepEqual(actual instanceof Error ? actual.message : actual, revive(c.expected));
			pass++;
		} catch (e) {
			report('custom.json', c, e);
		}
	}
}

(async () => {
	runParseSuites();
	runUnparseSuite();
	await runCustomSuite();
	console.log(`conformance (${path.basename(path.resolve(process.argv[2]))}): ${pass} passed, ${fail} failed, ${skip} skipped`);
	process.exit(fail ? 1 : 0);
})();
