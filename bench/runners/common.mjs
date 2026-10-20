import { readFileSync } from 'node:fs';
import { performance } from 'node:perf_hooks';

/** FNV-1a (32-bit) over UTF-8 bytes; the Rust runner computes the same. */
export function fnv1a(str) {
	const bytes = Buffer.from(str, 'utf8');
	let h = 0x811c9dc5;
	for (let i = 0; i < bytes.length; i++) h = Math.imul(h ^ bytes[i], 0x01000193);
	return (h >>> 0).toString(16).padStart(8, '0');
}

/**
 * Usage: node <runner> <file> <config-json> <iterations>
 * `config.mode` picks what is timed: `parse` (default), `step` (rows through a `step`
 * callback) or `unparse` (writing the parsed rows back to CSV).
 */
export function run(Papa) {
	const [file, configJson, iterations] = process.argv.slice(2);
	const input = readFileSync(file, 'utf8');
	const { mode = 'parse', ...config } = JSON.parse(configJson);

	let once;
	if (mode === 'step') {
		once = () => {
			const data = [];
			let errors = 0;
			Papa.parse(input, { ...config, step(r) { data.push(r.data); errors += r.errors.length; } });
			return { data, errors, checksumOf: data };
		};
	} else if (mode === 'unparse') {
		const { data } = Papa.parse(input, { ...config });
		once = () => {
			const csv = Papa.unparse(data);
			return { data, errors: 0, checksumOf: csv };
		};
	} else {
		once = () => {
			const r = Papa.parse(input, { ...config });
			return { data: r.data, errors: r.errors.length, checksumOf: r.data };
		};
	}

	once(); // warm-up (JIT, wasm instantiation)
	const times = [];
	let result;
	for (let i = 0; i < Number(iterations); i++) {
		result = undefined;
		const t0 = performance.now();
		result = once();
		times.push(performance.now() - t0);
	}
	const text = typeof result.checksumOf === 'string' ? result.checksumOf : JSON.stringify(result.checksumOf);
	console.log(JSON.stringify({
		times,
		rows: result.data.length,
		errors: result.errors,
		checksum: fnv1a(text),
	}));
}
