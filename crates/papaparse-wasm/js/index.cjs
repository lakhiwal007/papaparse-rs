'use strict';
// PapaParse-compatible entry point for the wasm build (Node / CommonJS).

const wasm = require('./papaparse_wasm.js');
const { createPapa } = require('./papa.cjs');

/** Moves a tape out of wasm memory and frees it deterministically. */
function takeTape(tape) {
	try {
		return {
			tags: tape.take_tags(),
			spans: tape.take_spans(),
			numbers: tape.take_numbers(),
			rowLens: tape.take_row_lens(),
			strings: tape.take_strings(),
			stringSpans: tape.take_string_spans(),
			info: tape.take_info(),
		};
	} finally {
		tape.free();
	}
}

/** Moves a batch of step rows out of wasm memory. */
function takeRows(rows) {
	try {
		return {
			tags: rows.take_tags(),
			spans: rows.take_spans(),
			numbers: rows.take_numbers(),
			rowLens: rows.take_row_lens(),
			strings: rows.take_strings(),
			stringSpans: rows.take_string_spans(),
			cursors: rows.take_cursors(),
			charIndexes: rows.take_char_indexes(),
			errors: rows.take_errors(),
			meta: rows.take_meta(),
		};
	} finally {
		rows.free();
	}
}

module.exports = createPapa({
	parseTape: (input, config) => takeTape(wasm.parse_tape(input, config)),
	newSession(config) {
		const session = new wasm.ParseSession(config);
		return {
			parse: (input, base, ignoreLastRow, transformHeader, onRows) =>
				takeTape(session.parse(input, base, ignoreLastRow, transformHeader,
					onRows && ((rows) => onRows(takeRows(rows))))),
			free: () => session.free(),
		};
	},
	guessLineEndings: wasm.guess_line_endings,
	unparse: wasm.unparse,
});
