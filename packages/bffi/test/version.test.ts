/**
 * Tests for the runtime Bun version gate: the numeric comparator
 * (`1.10.0` must be NEWER than `1.4.2` - the regression a string
 * comparison would fail), prerelease suffixes, the no-runtime case,
 * and the assert/problem-message shapes.
 *
 * Run with `bun test packages/bffi`.
 */
import { describe, expect, test } from "bun:test";

import {
  assertBunVersion,
  bunVersionProblem,
  bunVersionSatisfies,
  MIN_BUN_VERSION,
} from "../src/index.ts";

describe("bunVersionSatisfies", () => {
  test("the minimum itself satisfies", () => {
    expect(bunVersionSatisfies("1.4.2")).toBeTrue();
    expect(bunVersionSatisfies("1.4")).toBeFalse();
  });

  test("older versions are rejected - including the former 1.4.0 floor", () => {
    expect(bunVersionSatisfies("1.4.1")).toBeFalse();
    expect(bunVersionSatisfies("1.4.0")).toBeFalse();
    expect(bunVersionSatisfies("1.3.9")).toBeFalse();
    expect(bunVersionSatisfies("1.3")).toBeFalse();
    expect(bunVersionSatisfies("1.0.0")).toBeFalse();
    expect(bunVersionSatisfies("0.9.1")).toBeFalse();
  });

  test("numeric comparison: 1.10.0 is NEWER than 1.4.2", () => {
    expect(bunVersionSatisfies("1.10.0")).toBeTrue();
    expect(bunVersionSatisfies("1.9.9")).toBeTrue();
    expect(bunVersionSatisfies("1.5.0")).toBeTrue();
    expect(bunVersionSatisfies("2.0.0")).toBeTrue();
  });

  test("prerelease suffixes compare by the leading triple", () => {
    expect(bunVersionSatisfies("1.4.2-canary.12")).toBeTrue();
    expect(bunVersionSatisfies("1.5.0-beta.1")).toBeTrue();
    expect(bunVersionSatisfies("1.4.1-canary.1")).toBeFalse();
    expect(bunVersionSatisfies("1.3.0-canary.1")).toBeFalse();
  });

  test("undefined and garbage are rejected", () => {
    expect(bunVersionSatisfies(undefined)).toBeFalse();
    expect(bunVersionSatisfies("")).toBeFalse();
    expect(bunVersionSatisfies("unknown")).toBeFalse();
  });

  test("a custom minimum is honored", () => {
    expect(bunVersionSatisfies("1.4.2", "1.5.0")).toBeFalse();
    expect(bunVersionSatisfies("1.5.0", "1.5.0")).toBeTrue();
    // The former 1.4.0 floor stays supported as an explicit argument.
    expect(bunVersionSatisfies("1.4.0", "1.4.0")).toBeTrue();
  });

  test("MIN_BUN_VERSION is the AGENTS.md floor", () => {
    expect(MIN_BUN_VERSION).toBe("1.4.2");
  });
});

describe("bunVersionProblem / assertBunVersion", () => {
  test("a satisfying version yields no problem and no throw", () => {
    expect(bunVersionProblem("1.4.2")).toBeUndefined();
    expect(() => assertBunVersion("1.4.2")).not.toThrow();
    expect(() => assertBunVersion("1.4.5")).not.toThrow();
  });

  test("the former floor is now below the gate", () => {
    expect(bunVersionProblem("1.4.0")).toBe(
      "requires Bun >= 1.4.2; found 1.4.0. Upgrade Bun: https://bun.sh",
    );
  });

  test("an old version produces an upgrade message", () => {
    expect(bunVersionProblem("1.3.2")).toBe(
      "requires Bun >= 1.4.2; found 1.3.2. Upgrade Bun: https://bun.sh",
    );
    expect(() => assertBunVersion("1.3.2")).toThrow(
      /@z2net\/bffi requires Bun >= 1\.4\.2; found 1\.3\.2/,
    );
  });

  test("a missing runtime produces the no-Bun message", () => {
    expect(bunVersionProblem(undefined)).toBe(
      "requires Bun >= 1.4.2; no Bun runtime detected",
    );
  });
});
