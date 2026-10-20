'use strict';
// PapaParse-compatible entry point for the native Node-API addon.

const native = require('./binding.cjs');
const { createPapa } = require('./papa.cjs');

module.exports = createPapa({
	parseTape: native.parseTape,
	newSession(config) {
		const session = new native.ParseSession(config);
		return {
			parse: (input, base, ignoreLastRow, transformHeader, onRows) =>
				session.parse(input, base, ignoreLastRow, transformHeader, onRows),
			free() {}, // garbage-collected with the JS object
		};
	},
	guessLineEndings: native.guessLineEndings,
	unparse: native.unparse,
});
