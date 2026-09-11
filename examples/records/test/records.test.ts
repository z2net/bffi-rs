/**
 * End-to-end test of the FULL `@z2net/bffi` pipeline on the records
 * example: ONE call - cargo build -> loader JSON -> api.gen
 * generation -> binary resolution -> dlopen - then the B1 composite
 * round-trips: a record built from primitives, record parameters and
 * returns, record sequences in and numbers/strings out, and the enum
 * result channel (including its domain error).
 *
 * Run with `bun test examples/records` from the REPO ROOT (the root
 * tsconfig paths resolve `@z2net/bffi`).
 */
import { describe, expect, test } from "bun:test";

import { bffi } from "@z2net/bffi";
import type { Api } from "../.bffi/api.gen.ts";

/** The generated Api type derives the Sample shape from the schema
 * literal: `axis` is the Axis variant union, `label` a string. */
type Sample = Parameters<Api["recenter"]>[0];

let api: Api;

describe("records through the full pipeline", () => {
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

  test("a record built from primitives crosses back as an object", () => {
    const sample = api.make_sample(1.5, 10, "alpha", "Vertical");
    expect(sample).toEqual({
      at: 1.5,
      weight: 10,
      label: "alpha",
      axis: "Vertical",
    });
  });

  test("record parameters in, records out", () => {
    const input: Sample = {
      at: 1.5,
      weight: 10,
      label: "alpha",
      axis: "Vertical",
    };
    expect(api.recenter(input, 2.5)).toEqual({
      at: 4,
      weight: 10,
      label: "alpha",
      axis: "Vertical",
    });
  });

  test("record sequences cross both ways", () => {
    const samples: Sample[] = [
      { at: 1.25, weight: 2, label: "a", axis: "Horizontal" },
      { at: -3.5, weight: 40, label: "b", axis: "Vertical" },
    ];
    expect(api.total_weight(samples)).toBe(42n);
    expect(api.distances(samples)).toEqual([1.25, 3.5]);
    expect(api.labels(samples)).toEqual(["a", "b"]);
  });

  test("the enum result channel", () => {
    const samples: Sample[] = [
      { at: 0, weight: 1, label: "x", axis: "Vertical" },
    ];
    expect(api.classify(samples)).toBe("Vertical");
  });

  test("empty input reports the domain error", () => {
    expect(() => api.classify([])).toThrow(/no samples/);
  });

  test("strict composite kinds reject wrong shapes", () => {
    // @ts-expect-error: a number in an i32 slot must not pass silently.
    expect(() => api.make_sample(1, "ten", "alpha", "Vertical")).toThrow(TypeError);
    // @ts-expect-error: an unknown variant is rejected at the type level too.
    expect(() => api.make_sample(1, 10, "alpha", "Diagonal")).toThrow(/unknown Axis variant/);
  });
});
