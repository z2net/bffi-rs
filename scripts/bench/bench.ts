/**
 * The JS boundary benchmark: N calls of `add(1, 2)` through the
 * reference cdylib (`bffi-native`), comparing the SPECIALIZED
 * generated loader (`crates/bffi-native/.bffi/api.gen.ts` ->
 * `createApiFromJson`: hoisted symbols, preallocated out slots,
 * inline argument handling) against the GENERIC loader (`createApi`
 * from `@z2net/bffi` over the same `moduleJson`).
 *
 * Run from the repo root (see scripts/bench/README.md):
 *
 *   bun scripts/bench/bench.ts [path-to-cdylib] [calls]
 *
 * No dependencies: plain console table.
 */

import path from "node:path";

import { createApi } from "@z2net/bffi";
import process from "node:process";
import {
  createApiFromJson,
  moduleJson,
} from "../../crates/bffi-native/.bffi/api.gen.ts";

/** The cargo cdylib name for this platform. */
function defaultLibraryName(): string {
  switch (process.platform) {
    case "win32":
      return "bffi_native.dll";
    case "darwin":
      return "libbffi_native.dylib";
    default:
      return "libbffi_native.so";
  }
}

const repoRoot = path.join(import.meta.dir, "..", "..");
const libraryPath =
  process.argv[2] ??
  path.join(repoRoot, "target", "release", defaultLibraryName());
const calls = Number(process.argv[3] ?? 1_000_000);
const warmupCalls = 50_000;
const a = 1;
const b = 2;

if (!Number.isFinite(calls) || calls <= 0) {
  console.error(`invalid call count: ${String(process.argv[3])}`);
  process.exit(1);
}

const specialized = createApiFromJson(libraryPath);
const generic = createApi(moduleJson, libraryPath);

// Correctness gate before any timing.
const expected = a + b;
const loaders: Array<[string, (x: number, y: number) => number]> = [
  ["specialized", specialized.add],
  ["generic", generic.add],
];
for (const [label, add] of loaders) {
  const got = add(a, b);
  if (got !== expected) {
    console.error(
      `${label}: add(${String(a)}, ${String(b)}) returned ${String(got)}, expected ${String(expected)}`,
    );
    process.exit(1);
  }
}

/** Warmup + one timed pass; returns total ms and M calls/s. */
function bench(add: (x: number, y: number) => number): { ms: number; rate: number } {
  for (let i = 0; i < warmupCalls; i++) {
    add(a, b);
  }
  const start = performance.now();
  for (let i = 0; i < calls; i++) {
    add(a, b);
  }
  const ms = performance.now() - start;
  return { ms, rate: calls / ms / 1000 };
}

const results = loaders.map(([label, add]) => ({
  label,
  ...bench(add),
}));
const baseline = results[0]?.rate ?? 1;

console.info(`bffi JS boundary benchmark - add(${String(a)}, ${String(b)})`);
console.info(`library : ${libraryPath}`);
console.info(
  `calls   : ${String(calls)} per loader (warmup ${String(warmupCalls)})`,
);
console.info("");
console.info(
  "  loader          total ms    M calls/s    vs specialized",
);
for (const result of results) {
  const ratio = result.rate / baseline;
  console.info(
    "  " +
      result.label.padEnd(16) +
      result.ms.toFixed(1).padStart(8) +
      result.rate.toFixed(2).padStart(13) +
      (ratio.toFixed(2) + "x").padStart(18),
  );
}
console.info("");
// No environment details are printed on purpose: published benchmark
// numbers must not leak or imply a particular maintainer machine -
// the README cites the CI runner the workflow ran on.
