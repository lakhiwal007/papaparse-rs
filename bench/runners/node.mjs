import { createRequire } from 'node:module';
import { run } from './common.mjs';

run(createRequire(import.meta.url)('../pkg-node/index.cjs'));
