'use strict';
// Streaming and callback behaviour on inputs large enough to span many step batches,
// compared event by event with PapaParse: pause / resume and abort at arbitrary rows,
// `transform` call counts, `chunkSize`, Node.js readable streams and NODE_STREAM_INPUT.
// Usage: node streaming.cjs <pkg-dir>
const assert = require('node:assert/strict');
const path = require('node:path');
const { Readable } = require('node:stream');
const Papa = require(require.resolve('papaparse', { paths: [path.join(__dirname, '../../bench')] }));
const Ours = require(path.resolve(process.argv[2], 'index.cjs'));

function makeInput(rows) {
	const lines = ['id,name,"note, quoted",amount'];
	for (let i = 0; i < rows; i++) {
		const note = i % 7 === 0 ? `"multi\nline ${i}, with ""quotes"""` : `note ${i}`;
		const extra = i % 97 === 0 ? ',extra' : '';
		lines.push(`${i},n${i % 13},${note},${(i * 1.25).toFixed(2)}${extra}`);
	}
	return lines.join('\n') + '\n';
}

/**
 * Runs `Papa.parse` with recording callbacks and resolves with the event trace. Without
 * pauses a string parses synchronously, so the trace is complete when `parse` returns (and
 * PapaParse skips `complete` when `preview` stops a step parse); with pauses, at `complete`.
 */
function trace(lib, input, config, { pauseEvery = 0, abortAt = -1 } = {}) {
	return new Promise((resolve, reject) => {
		const events = [];
		let transforms = 0, steps = 0;
		const timer = pauseEvery ? setTimeout(() => reject(new Error('no complete')), 20000) : null;
		const done = (results) => {
			events.push(['complete', structuredClone(results)]);
			if (pauseEvery) {
				clearTimeout(timer);
				resolve({ events, transforms });
			}
		};
		lib.parse(input, {
			...config,
			transform: config.transform ? (v, f) => { transforms++; return config.transform(v, f); } : undefined,
			step(results, handle) {
				events.push(['step', structuredClone(results)]);
				steps++;
				if (steps - 1 === abortAt) return handle.abort();
				if (pauseEvery && steps % pauseEvery === 0) {
					handle.pause();
					setImmediate(() => handle.resume());
				}
			},
			complete: done,
		});
		if (!pauseEvery) resolve({ events, transforms });
	});
}

const checks = [];
function check(name, fn) { checks.push([name, fn]); }

const big = makeInput(3000);
for (const config of [
	{ header: true },
	{ header: true, dynamicTyping: true, chunkSize: 4096 },
	{ dynamicTyping: true, transform: (v, f) => (f === 2 ? v.toUpperCase() : v) },
	{ header: true, skipEmptyLines: 'greedy', preview: 1700 },
]) {
	const label = JSON.stringify(config, (k, v) => (typeof v === 'function' ? 'fn' : v));
	check(`step ${label}`, async () => {
		const [a, b] = await Promise.all([trace(Papa, big, config), trace(Ours, big, config)]);
		assert.deepEqual(b, a);
	});
	check(`abort mid-batch ${label}`, async () => {
		const [a, b] = await Promise.all([trace(Papa, big, config, { abortAt: 777 }), trace(Ours, big, config, { abortAt: 777 })]);
		assert.deepEqual(b.events.length, a.events.length);
		assert.equal(b.transforms, a.transforms, 'transform calls');
		assert.deepEqual(b.events.at(-2), a.events.at(-2));
	});
}

// Includes PapaParse's quirk of reporting cursors from a stale base index after a resume.
check('pause / resume mid-batch', async () => {
	const config = { header: true, dynamicTyping: true };
	const [a, b] = await Promise.all([trace(Papa, big, config, { pauseEvery: 600 }), trace(Ours, big, config, { pauseEvery: 600 })]);
	assert.deepEqual(b, a);
});

check('Node.js readable stream', async () => {
	const run = (lib) => new Promise((resolve) => {
		const rows = [];
		const pieces = big.match(/[\s\S]{1,1000}/g);
		lib.parse(Readable.from(pieces.map((p) => Buffer.from(p))), {
			header: true,
			step: (r) => rows.push([r.data, r.errors]),
			complete: () => resolve(rows),
		});
	});
	const [a, b] = await Promise.all([run(Papa), run(Ours)]);
	assert.equal(b.length, 3000);
	assert.deepEqual(b, a);
});

check('NODE_STREAM_INPUT duplex', async () => {
	const run = (lib) => new Promise((resolve, reject) => {
		const rows = [];
		const duplex = lib.parse(lib.NODE_STREAM_INPUT, { header: true, dynamicTyping: true });
		duplex.on('data', (row) => rows.push(row));
		duplex.on('end', () => resolve(rows));
		duplex.on('error', reject);
		Readable.from(big.match(/[\s\S]{1,777}/g)).pipe(duplex);
	});
	const [a, b] = await Promise.all([run(Papa), run(Ours)]);
	assert.equal(b.length, 3000);
	assert.deepEqual(b, a);
});

check('chunk callback with chunkSize', async () => {
	const run = (lib) => new Promise((resolve) => {
		const chunks = [];
		lib.parse(big, {
			chunkSize: 5000,
			chunk: (r) => chunks.push(structuredClone(r)),
			complete: () => resolve(chunks),
		});
	});
	const [a, b] = await Promise.all([run(Papa), run(Ours)]);
	assert.deepEqual(b, a);
});

(async () => {
	let failed = 0;
	for (const [name, fn] of checks) {
		try {
			await fn();
		} catch (e) {
			failed++;
			console.log(`FAIL ${name}\n  ${String(e.message).split('\n').slice(0, 14).join('\n  ')}`);
		}
	}
	console.log(`streaming (${path.basename(path.resolve(process.argv[2]))}): ${checks.length - failed} passed, ${failed} failed`);
	process.exit(failed ? 1 : 0);
})();
