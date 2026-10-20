'use strict';
// Shared by the wasm and Node-API packages: turns a parse tape (see
// crates/papaparse-tape/src/tape.rs) into the `{ data, errors, meta }` object `Papa.parse` returns.

// Must match `tag` in src/tape.rs.
const INPUT = 0, SIDE = 1, NUMBER = 2, TRUE = 3, FALSE = 4, NULL = 5, DATE_INPUT = 6, DATE_SIDE = 7;

/**
 * Returns a function that decodes the tape's next row on each call.
 * @param {string} input the string that was parsed
 * @param {{tags: Uint8Array, spans: Uint32Array, numbers: Float64Array, rowLens: Uint32Array,
 *          strings: string, stringSpans: Uint32Array}} t
 * @param {string[] | null | undefined} fields header names: rows become objects
 * @param {((value: unknown, field: string | number) => unknown) | null} [post] applied to every
 *        cell (JS `transform` and dynamic typing, when those run on the JS side)
 */
function rowDecoder(input, t, fields, post) {
	const { tags, spans, numbers, rowLens, strings: side, stringSpans: sideSpans } = t;
	let i = 0, s = 0, n = 0, ss = 0, r = 0;
	function cell() {
		switch (tags[i++]) {
			case INPUT: { const v = input.substring(spans[s], spans[s + 1]); s += 2; return v; }
			case SIDE: { const v = side.substring(sideSpans[ss], sideSpans[ss + 1]); ss += 2; return v; }
			case NUMBER: return numbers[n++];
			case TRUE: return true;
			case FALSE: return false;
			case NULL: return null;
			case DATE_INPUT: { const v = new Date(input.substring(spans[s], spans[s + 1])); s += 2; return v; }
			case DATE_SIDE: { const v = new Date(side.substring(sideSpans[ss], sideSpans[ss + 1])); ss += 2; return v; }
			default: throw new Error('papaparse: corrupt tape');
		}
	}

	if (fields) {
		const width = fields.length;
		return function nextRecord() {
			const len = rowLens[r++];
			const obj = {};
			for (let j = 0; j < len; j++) {
				const field = j < width ? fields[j] : '__parsed_extra';
				const v = post ? post(cell(), field) : cell();
				if (j < width) obj[field] = v;
				else (obj.__parsed_extra || (obj.__parsed_extra = [])).push(v);
			}
			return obj;
		};
	}
	return function nextRow() {
		const len = rowLens[r++];
		const row = new Array(len);
		for (let j = 0; j < len; j++) row[j] = post ? post(cell(), j) : cell();
		return row;
	};
}

/**
 * Turns a parse tape into `{ data, errors, meta }`.
 * @param {string} input the string that was parsed
 * @param t the tape, plus `info`: `{"errors":[...],"meta":{...}}` as JSON
 * @param post see `rowDecoder`
 */
function decode(input, t, post) {
	const info = JSON.parse(t.info);
	const next = rowDecoder(input, t, info.meta.fields, post);
	const rows = t.rowLens.length;
	const data = new Array(rows);
	for (let r = 0; r < rows; r++) data[r] = next();
	return { data, errors: info.errors, meta: info.meta };
}

module.exports = { decode, rowDecoder };
