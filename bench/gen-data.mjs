// Generates deterministic benchmark datasets into bench/data/.
import { mkdirSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const out = join(dirname(fileURLToPath(import.meta.url)), 'data');
mkdirSync(out, { recursive: true });

// mulberry32: tiny seeded PRNG so every run produces identical files.
function rng(seed) {
	return () => {
		seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
		let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
		t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}
const words = ['alpha', 'bravo', 'charlie', 'delta', 'echo', 'foxtrot', 'golf', 'hotel', 'india', 'juliet', 'kilo', 'lima'];

function write(name, rows, makeRow, header) {
	const r = rng(42);
	const lines = [header];
	for (let i = 0; i < rows; i++) lines.push(makeRow(r, i));
	const text = lines.join('\n') + '\n';
	writeFileSync(join(out, name), text);
	console.log(`${name}: ${rows} rows, ${(Buffer.byteLength(text) / 1e6).toFixed(1)} MB`);
}

// Typical machine-generated export: no quotes, numbers and short strings.
write('simple.csv', 500_000, (r, i) => [
	i, words[(r() * words.length) | 0], (r() * 1e4).toFixed(2), (r() * 100) | 0,
	r() < 0.5 ? 'true' : 'false', words[(r() * words.length) | 0] + '_' + ((r() * 1000) | 0),
].join(','), 'id,name,price,qty,active,tag');

// Spreadsheet-style export: quoted text with commas, escaped quotes and line breaks.
write('quoted.csv', 250_000, (r, i) => {
	const w = () => words[(r() * words.length) | 0];
	const note = r() < 0.2 ? `"${w()} says ""hi"", ${w()}\n${w()}"` : `"${w()}, ${w()} and ${w()}"`;
	return [i, `"${w()} ${w()}"`, note, (r() * 1e4).toFixed(2), `"${w()}"`].join(',');
}, 'id,name,note,amount,category');

// Adversarial: one quoted column, no delimiter anywhere. JS indexOf rescans make this quadratic.
write('pathological.csv', 200_000, () => '"x"', 'col');
