'use strict';
// Loads the native addon: `papaparse.node` next to this file (a local build), or else the
// prebuilt package for this platform, which npm installs as an optional dependency
// (see scripts/npm-packages.mjs for the package names).

const { existsSync, readFileSync } = require('node:fs');
const { join } = require('node:path');

function isMusl() {
	try {
		return readFileSync('/usr/bin/ldd', 'utf8').includes('musl');
	} catch {
		const report = process.report && process.report.getReport();
		return !(report && report.header && report.header.glibcVersionRuntime);
	}
}

/** Name of the package holding the prebuilt addon for this platform. */
function platformPackage() {
	const { platform, arch } = process;
	const abi = platform === 'linux' ? (isMusl() ? '-musl' : '-gnu') : platform === 'win32' ? '-msvc' : '';
	return `papaparse-node-${platform}-${arch}${abi}`;
}

const local = join(__dirname, 'papaparse.node');
if (existsSync(local)) {
	module.exports = require(local);
} else {
	const name = platformPackage();
	try {
		module.exports = require(name);
	} catch (cause) {
		throw new Error(
			`papaparse-node: no prebuilt addon for ${process.platform}-${process.arch} (package "${name}"). ` +
			'Use papaparse-wasm, which runs everywhere, or build the addon from source (scripts/build-node.sh).',
			{ cause },
		);
	}
}
