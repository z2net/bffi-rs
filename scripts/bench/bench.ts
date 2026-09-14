/**
 * The JS boundary benchmark: N calls of `sum_to(1000n)` through the
 * workers example cdylib, comparing the SPECIALIZED generated loader
 * (`examples/workers/.bffi/api.gen.ts` -> `createApiFromJson`: hoisted
 * symbols, preallocated out slots, inline argument handling) against
 * the GENERIC loader (`createApi` from `@z2net/bffi` over the same
 * `moduleJson`).
 *
 * Run from the repo root (see scripts/bench/README.md):
 *
 *   bun scripts/bench/bench.ts [path-to-cdylib] [calls]
 *
 * No dependencies: plain console table.
 */

import os from "node:os";
import path from "node:path";

import { createApi } from "@z2net/bffi";
import {
  createApiFromJson,
  moduleJson,
} from "../../examples/workers/.bffi/api.gen.ts";

/** The cargo cdylib name for this platform. */
function defaultLibraryName(): string {
  switch (os.platform()) {
    case "win32":
      return "bffi_example_workers.dll";
    case "darwin":
      return "libbffi_example_workers.dylib";
    default:
      return "libbffi_example_workers.so";
  }
}

const repoRoot = path.join(import.meta.dir, "..", "..");
const libraryPath =
  process.argv[2] ??
  path.join(repoRoot, "target", "release", defaultLibraryName());
const calls = Number(process.argv[3] ?? 1_000_000);
const warmupCalls = 50_000;
const arg = 1000n;

if (!Number.isFinite(calls) || calls <= 0) {
  console.error(`invalid call count: ${String(process.argv[3])}`);
  process.exit(1);
}

const specialized = createApiFromJson(libraryPath);
const generic = createApi(moduleJson, libraryPath);

// Correctness gate before any timing.
const expected = (arg * (arg + 1n)) / 2n;
const loaders: Array<[string, (n: bigint) => bigint]> = [
  ["specialized", specialized.sum_to],
  ["generic", generic.sum_to],
];
for (const [label, sumTo] of loaders) {
  const got = sumTo(arg);
  if (got !== expected) {
    console.error(
      `${label}: sum_to(${String(arg)}) returned ${String(got)}, expected ${String(expected)}`,
    );
    process.exit(1);
  }
}

/** Warmup + one timed pass; returns total ms and M calls/s. */
function bench(sumTo: (n: bigint) => bigint): { ms: number; rate: number } {
  for (let i = 0; i < warmupCalls; i++) {
    sumTo(arg);
  }
  const start = performance.now();
  for (let i = 0; i < calls; i++) {
    sumTo(arg);
  }
  const ms = performance.now() - start;
  return { ms, rate: calls / ms / 1000 };
}

const results = loaders.map(([label, sumTo]) => ({
  label,
  ...bench(sumTo),
}));
const baseline = results[0]?.rate ?? 1;

console.info(`bffi JS boundary benchmark - sum_to(${String(arg)})`);
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

const cpu = os.cpus()[0]?.model.trim() ?? "unknown cpu";
console.info(
  `measured on ${cpu}, ${os.type()} ${os.release()} ${os.arch()}, bun ${String(Bun.version)}`,
);
