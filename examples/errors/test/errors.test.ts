/**
 * End-to-end test of the FULL `@z2net/bffi` pipeline on the errors
 * example: ONE call - cargo build -> loader JSON -> api.gen
 * generation -> binary resolution -> dlopen - then the B3 typed
 * error surface: user codes in the thrown error, variant names as
 * e.name, payload records as e.payload, and the two legacy channels
 * (String ad-hoc errors and explicit framework errors).
 *
 * Run with `bun test examples/errors` from the REPO ROOT (the root
 * tsconfig paths resolve `@z2net/bffi`).
 */
import { describe, expect, test } from "bun:test";

import { bffi } from "@z2net/bffi";
import type { Api } from "../.bffi/api.gen.ts";

let api: Api;

/** The user code range reserved for #[derive(BffiError)] enums. */
const NOT_FOUND = 0x1001;
const INVALID_AGE = 0x1002;

describe("typed errors through the full pipeline", () => {
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

  test("a successful call carries no error", () => {
    const user = api.find_user(1n);
    expect(user).toEqual({ id: 1n, name: "ada" });
  });

  test("the derived user code crosses in the thrown error", () => {
    try {
      api.find_user(99n);
      throw new Error("must have thrown");
    } catch (error) {
      expect((error as Error & { code: number }).code).toBe(NOT_FOUND);
      expect((error as Error).name).toBe("NotFound");
      expect((error as Error).message).toContain("NotFound");
    }
  });

  test("variant fields arrive as the payload object", () => {
    try {
      api.find_user(77n);
      throw new Error("must have thrown");
    } catch (error) {
      const payload = (error as Error & { payload: unknown }).payload;
      expect(payload).toEqual([77n]);
    }
  });

  test("INVALID_AGE carries value and bound", () => {
    try {
      api.validate_age(-5);
      throw new Error("must have thrown");
    } catch (error) {
      expect((error as Error & { code: number }).code).toBe(INVALID_AGE);
      expect((error as Error).name).toBe("InvalidAge");
      const payload = (error as Error & { payload: unknown }).payload;
      expect(payload).toEqual([-5, 0]);
    }
  });

  test("a valid age passes", () => {
    expect(api.validate_age(33)).toBe(true);
  });

  test("String errors map to DomainError (13)", () => {
    try {
      api.ad_hoc();
      throw new Error("must have thrown");
    } catch (error) {
      expect((error as Error & { code: number }).code).toBe(13);
      expect((error as Error).message).toBe("ad-hoc failure");
    }
  });

  test("explicit framework errors keep their code and name", () => {
    try {
      api.framework_error();
      throw new Error("must have thrown");
    } catch (error) {
      expect((error as Error & { code: number }).code).toBe(11);
      expect((error as Error).message).toContain("explicit framework error");
    }
  });
});
