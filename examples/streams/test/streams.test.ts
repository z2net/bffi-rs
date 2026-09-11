/**
 * End-to-end test of the FULL `@z2net/bffi` pipeline on the streams
 * example: ONE call - cargo build -> loader JSON -> api.gen
 * generation -> binary resolution -> dlopen - then the B2 stream
 * round-trips: numeric streams, record streams (the B1 composites
 * compose), string transforms, early exit through break (which
 * releases the native iterator), and for-await ergonomics.
 *
 * Run with `bun test examples/streams` from the REPO ROOT (the root
 * tsconfig paths resolve `@z2net/bffi`).
 */
import { describe, expect, test } from "bun:test";

import { bffi } from "@z2net/bffi";
import type { Api } from "../.bffi/api.gen.ts";

/** The generated Api derives the Sample shape from the schema
 * literal; the compile-level assertions below pin it. */
type Sample = {
  at: number;
  label: string;
  axis: "Horizontal" | "Vertical";
};

let api: Api;

/** Collects every item of an async iterable. */
async function collect<T>(stream: AsyncIterableIterator<T>): Promise<T[]> {
  const items: T[] = [];
  for await (const item of stream) {
    items.push(item);
  }
  return items;
}

describe("streams through the full pipeline", () => {
  // A COLD release build (LTO, whole workspace) takes minutes; bun's
  // default per-test timeout is 5s.
  test(
    "the pipeline builds, generates and loads the module",
    async () => {
      api = await bffi({ config: `${import.meta.dir}/../.bffi/bffi.json` });
      expect(api).toBeTypeOf("object");
    },
    600_000,
  );

  test("a numeric stream yields its whole range in order", async () => {
    expect(await collect(api.numbers(5))).toEqual([1, 2, 3, 4, 5]);
  });

  test("an empty stream yields nothing", async () => {
    expect(await collect(api.numbers(0))).toEqual([]);
  });

  test("record streams compose with the B1 composites", async () => {
    const items = await collect(api.samples(3));
    expect(items).toEqual([
      { at: 0, label: "station-0", axis: "Horizontal" },
      { at: 1, label: "station-1", axis: "Vertical" },
      { at: 2, label: "station-2", axis: "Horizontal" },
    ]);
    // Compile-level: the generated item type matches the shape.
    const first: Sample = items[0] as Sample;
    expect(first.label).toBe("station-0");
  });

  test("string transforms stream as strings", async () => {
    expect(await collect(api.fizzbuzz(15))).toEqual([
      "1", "2", "fizz", "4", "buzz", "fizz", "7", "8", "fizz", "buzz",
      "11", "fizz", "13", "14", "fizzbuzz",
    ]);
  });

  test("an early break releases the native iterator", async () => {
    const seen: number[] = [];
    for await (const n of api.numbers(1000)) {
      seen.push(n);
      if (seen.length === 3) {
        break;
      }
    }
    expect(seen).toEqual([1, 2, 3]);
  });

  test("stream types are exact (compile-level)", async () => {
    const first = (await collect(api.numbers(1)))[0];
    expect(first).toBe(1);
    // The generated Api types numbers as AsyncIterableIterator<number>:
    // every element is a number.
    const items: number[] = await collect(api.numbers(2));
    expect(items.every((n) => typeof n === "number")).toBe(true);
  });

  test("a slow push producer streams with backpressure", async () => {
    const started = Date.now();
    const items = await collect(api.readings(4));
    const elapsed = Date.now() - started;
    expect(items).toEqual([0, 1, 2, 3]);
    // Four 5 ms producer ticks must have actually elapsed.
    expect(elapsed).toBeGreaterThanOrEqual(15);
  });

  test("Result items arrive as values: T | Error", async () => {
    // Even indexes are values (exact u64 bigints), odd indexes are
    // error items - real Error instances yielded by the iterator.
    const items = await collect(api.flaky(4));
    expect(items).toHaveLength(4);
    expect(items[0]).toBe(0n);
    expect(items[1]).toBeInstanceOf(Error);
    expect((items[1] as Error).message).toContain("odd index 1");
    expect(items[2]).toBe(2n);
    expect(items[3]).toBeInstanceOf(Error);
    expect((items[3] as Error).message).toContain("odd index 3");
  });

  test("u64 items are exact bigints", async () => {
    const items = await collect(api.big_values(3));
    expect(items).toEqual([
      18446744073709551615n,
      18446744073709551614n,
      18446744073709551613n,
    ]);
    expect(items.every((n) => typeof n === "bigint")).toBe(true);
  });
});
