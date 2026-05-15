/// <reference types="node" />
import { readFileSync } from 'node:fs';
import { bench, describe } from 'vitest';
import { decode, encode } from '../src/index.js';

// The benchmark needs a real-world .jslb file. Pass its path via env var:
//   JSLB_BENCH_FIXTURE=/path/to/file.jslb npm run bench
const fixturePath = process.env.JSLB_BENCH_FIXTURE;

if (!fixturePath) {
  describe.skip('encode/decode benchmarks (set JSLB_BENCH_FIXTURE to enable)', () => {
    bench('skipped', () => {});
  });
} else {
  // Copy into a fresh Uint8Array so byteOffset === 0 (decode requires 8-byte alignment).
  const raw = readFileSync(fixturePath);
  const buf = new Uint8Array(raw.byteLength);
  buf.set(raw);

  const decoded = decode(buf);

  describe(`fixture: ${fixturePath} (${(buf.byteLength / 1024 / 1024).toFixed(1)} MiB)`, () => {
    bench(
      'encode',
      () => {
        encode(decoded);
      },
      { iterations: 5, warmupIterations: 1 },
    );

    bench(
      'decode',
      () => {
        decode(buf);
      },
      { iterations: 5, warmupIterations: 1 },
    );
  });
}
