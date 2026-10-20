'use strict';
// Differential fuzzing: random inputs and configs, a built package vs PapaParse. Covers
// one-shot parsing, `step` / `chunk` / `complete` callbacks with `chunkSize` and `abort`,
// JS `transform` / `transformHeader` / `dynamicTyping` functions, and `unparse`.
// Usage: node differential.cjs <pkg-dir> [cases=20000] [seed=1]
const assert = require('node:assert/strict');
const path = require('node:path');
const Papa = require(require.resolve('papaparse', { paths: [path.join(__dirname, '../../bench')] }));
const Ours = require(path.resolve(process.argv[2], 'index.cjs'));
const cases = Number(process.argv[3] ?? 20000);
let seed = Number(process.argv[4] ?? 1);

function rnd() { // mulberry32
	seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
	let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
	t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
	return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}
const pick = (xs) => xs[Math.floor(rnd() * xs.length)];
// Alphabet weighted towards CSV-significant characters, plus multi-byte text.
const ALPHABET = ['a', 'b', '1', '2', '.', '-', 'e', ' ', ' ', ',', ',', ';', '\t', '|', '"', '"', "'", '\\', '\n', '\n', '\r', '\r\n', '#', '', 'é', '😀', 'true', 'FALSE', '2018-05-04T21:08:03.269Z', '\u{feff}'];

function randomInput() {
	const n = Math.floor(rnd() * 40);
	let s = '';
	for (let i = 0; i < n; i++) s += pick(ALPHABET);
	return s;
}
function randomConfig() {
	const c = {};
	if (rnd() < 0.5) c.delimiter = pick([',', ';', '\t', '|', ', ']);
	if (rnd() < 0.3) c.newline = pick(['\n', '\r', '\r\n']);
	if (rnd() < 0.2) c.quoteChar = pick(['"', "'"]);
	if (rnd() < 0.2) c.escapeChar = pick(['"', '\\']);
	if (rnd() < 0.4) c.header = true;
	if (rnd() < 0.4) c.dynamicTyping = true;
	if (rnd() < 0.2) c.preview = 1 + Math.floor(rnd() * 3);
	if (rnd() < 0.2) c.comments = pick(['#', true]);
	if (rnd() < 0.3) c.skipEmptyLines = pick([true, 'greedy']);
	if (rnd() < 0.1) c.skipFirstNLines = 1 + Math.floor(rnd() * 2);
	if (rnd() < 0.1) c.fastMode = pick([true, false]);
	return c;
}

/** Random callback choices for one case, applied identically to both libraries. */
function randomPlan() {
	const mode = pick(['plain', 'plain', 'step', 'chunk']);
	return {
		mode,
		abortAt: rnd() < 0.3 ? Math.floor(rnd() * 4) : -1,
		chunkSize: mode !== 'plain' && rnd() < 0.7 ? 1 + Math.floor(rnd() * 12) : undefined,
		complete: rnd() < 0.5,
	};
}

/** Installs the plan's callbacks on `config`; returns the trace they record. */
function install(config, plan) {
	const trace = [];
	let calls = 0;
	const record = (name) => (results, handle) => {
		trace.push([name, structuredClone(results)]);
		if (calls++ === plan.abortAt) handle.abort();
	};
	if (plan.mode === 'step') config.step = record('step');
	if (plan.mode === 'chunk') config.chunk = record('chunk');
	if (plan.chunkSize) config.chunkSize = plan.chunkSize;
	if (plan.complete) config.complete = (results) => trace.push(['complete', structuredClone(results)]);
	return trace;
}

function randomHooks(config) {
	const r = rnd();
	if (r < 0.15) config.transform = (v, f) => (typeof f === 'number' ? f % 2 : f.length % 2) ? v + '|' + f : v;
	else if (r < 0.25) config.transform = (v) => v.length > 2 ? Number(v) : v;
	if (rnd() < 0.15) config.dynamicTyping = (f) => (typeof f === 'number' ? f : f.length) % 2 === 0;
	if (rnd() < 0.15) config.transformHeader = (h, i) => i % 2 ? h.toUpperCase() : h + 'x';
	if (rnd() < 0.05) config.delimiter = (input) => input.includes(';') ? ';' : ',';
}

function parseCase(i) {
	const input = randomInput();
	const base = randomConfig();
	if (rnd() < 0.4) randomHooks(base);
	const plan = randomPlan();
	const theirs = { ...base }, ours = { ...base };
	const theirTrace = install(theirs, plan), ourTrace = install(ours, plan);
	const shown = { ...base, ...plan };
	for (const k of Object.keys(shown)) if (typeof shown[k] === 'function') shown[k] = String(shown[k]);
	return run(i, JSON.stringify(input) + ' ' + JSON.stringify(shown),
		() => [Papa.parse(input, theirs), theirTrace],
		() => [Ours.parse(input, ours), ourTrace]);
}

function randomValue() {
	const r = rnd();
	// Whole code points: lone surrogates can't cross into Rust strings (see docs/PORTING.md).
	if (r < 0.5) return Array.from(randomInput()).slice(0, 8).join('');
	if (r < 0.62) return pick([0, -1, 1.5, 1e21, 1e-7, -0.000001, 123456789.125, NaN, Infinity]);
	if (r < 0.7) return rnd() < 0.5;
	if (r < 0.78) return pick([null, undefined]);
	if (r < 0.86) return pick([new Date(0), new Date(NaN), new Date('2018-05-04T21:08:03.269Z')]);
	if (r < 0.9) return pick([['x', 1], { a: 1 }]);
	return pick(['=1+1', '+x', '-y', '@z', '\tq', ' lead', 'trail ', '']);
}

function unparseCase(i) {
	const keyed = rnd() < 0.5;
	const keys = ['a', 'b', 'c', 'a b', '"q"'].slice(0, 1 + Math.floor(rnd() * 4));
	const rows = Array.from({ length: Math.floor(rnd() * 5) }, () => {
		if (keyed) {
			const o = {};
			for (const k of keys) if (rnd() < 0.85) o[k] = randomValue();
			return o;
		}
		return Array.from({ length: Math.floor(rnd() * 4) }, randomValue);
	});
	let input = rows;
	if (rnd() < 0.25) input = { fields: rnd() < 0.7 ? keys : undefined, data: rows };
	const c = {};
	if (rnd() < 0.3) c.quotes = pick([true, false, [true, false, true], (v, col) => typeof v === 'string' && col % 2 === 0]);
	if (rnd() < 0.3) c.delimiter = pick([';', '\t', '|', ', ', '"', 'ab']);
	if (rnd() < 0.2) c.newline = pick(['\n', '\r', '$']);
	if (rnd() < 0.2) c.quoteChar = pick(["'", '|', '']);
	if (rnd() < 0.2) c.escapeChar = pick(['\\', "'", '"']);
	if (rnd() < 0.2) c.header = false;
	if (rnd() < 0.3) c.skipEmptyLines = pick([true, false, 'greedy']);
	if (rnd() < 0.15) c.columns = pick([['a', 'c'], ['b']]);
	if (rnd() < 0.25) c.escapeFormulae = pick([true, /^[=x]/, /a/g]);
	const shown = JSON.stringify([input, c], (k, v) => v instanceof RegExp || typeof v === 'function' ? String(v) : v === undefined ? '<undefined>' : v);
	// Fresh RegExp per side: `test` on a /g regex is stateful (`lastIndex`).
	const config = () => c.escapeFormulae instanceof RegExp ? { ...c, escapeFormulae: new RegExp(c.escapeFormulae) } : c;
	return run(i, 'unparse ' + shown,
		() => Papa.unparse(structuredCloneLoose(input), config()),
		() => Ours.unparse(structuredCloneLoose(input), config()));
}

/** Copies input (unparse may write `fields` into it) keeping Dates and undefined. */
function structuredCloneLoose(v) {
	if (v instanceof Date) return new Date(v.getTime());
	if (Array.isArray(v)) return v.map(structuredCloneLoose);
	if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, structuredCloneLoose(x)]));
	return v;
}

const silence = console.warn; // PapaParse warns on duplicate headers
let failures = 0, compared = 0, bothThrew = 0, jsCrashed = 0, loneSurrogates = 0;
// JSON.stringify escapes unpaired surrogates; they can't cross into Rust (see docs/PORTING.md).
const LONE_SURROGATE = /\\ud[89a-f][0-9a-f]{2}/i;
function run(i, label, theirs, ours) {
	let expected, expectedErr, actual, actualErr;
	console.warn = () => {};
	try { expected = theirs(); } catch (e) { expectedErr = e; }
	console.warn = silence;
	try { actual = ours(); } catch (e) { actualErr = e; }
	if (expectedErr || actualErr) {
		if (expectedErr && actualErr) { bothThrew++; return; }
		// PapaParse bugs such as `null.length` on some unparse inputs: we return output instead.
		if (expectedErr instanceof TypeError && !actualErr) { jsCrashed++; return; }
		failures++;
		if (failures <= (+process.env.SHOW || 10)) console.log(`MISMATCH (throw) #${i}: ${label}\n  js: ${expectedErr?.message}\n  rs: ${actualErr?.message}`);
		return;
	}
	try {
		assert.deepEqual(actual, expected);
		compared++;
	} catch (e) {
		if (LONE_SURROGATE.test(JSON.stringify(expected))) { loneSurrogates++; return; }
		failures++;
		if (failures <= (+process.env.SHOW || 10)) console.log(`MISMATCH #${i}: ${label}\n  js: ${JSON.stringify(expected)}\n  rs: ${JSON.stringify(actual)}`);
	}
}

for (let i = 0; i < cases; i++) {
	if (rnd() < 0.2) unparseCase(i);
	else parseCase(i);
}
console.log(`differential: ${compared} identical, ${bothThrew} both threw, ${jsCrashed} PapaParse-only TypeErrors, ${loneSurrogates} with unpaired surrogates, ${failures} mismatches (of ${cases})`);
process.exit(failures ? 1 : 0);
