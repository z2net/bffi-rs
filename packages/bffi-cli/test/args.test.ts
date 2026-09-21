/**
 * Tests for the argv parser: flags with values, boolean flags,
 * `--flag=value`, `--` terminator, and short aliases.
 *
 * Run with `bun test packages/bffi-cli`.
 */
import { describe, expect, test } from "bun:test";

import { flagBool, flagString, parseArgs } from "#cli/args.ts";

describe("parseArgs", () => {
  test("collects positionals", () => {
    const args = parseArgs(["codegen", "in.json", "-o", "out.ts"]);
    expect(args.positionals).toEqual(["codegen", "in.json"]);
  });

  test("long flags take the next token as value", () => {
    const args = parseArgs(["--out", "x.ts", "--runtime", "@z2net/bffi"]);
    expect(args.flags.out).toBe("x.ts");
    expect(args.flags.runtime).toBe("@z2net/bffi");
  });

  test("boolean flags without a following token are true", () => {
    const args = parseArgs(["build", "--skip-build", "--debug"]);
    expect(args.flags["skip-build"]).toBeTrue();
    expect(args.flags.debug).toBeTrue();
  });

  test("--flag=value form", () => {
    const args = parseArgs(["--out=x.ts"]);
    expect(args.flags.out).toBe("x.ts");
  });

  test("short aliases", () => {
    const args = parseArgs(["-o", "out.ts"]);
    expect(args.flags.o).toBe("out.ts");
  });

  test("-- stops flag parsing", () => {
    const args = parseArgs(["--", "--not-a-flag"]);
    expect(args.flags["not-a-flag"]).toBeUndefined();
    expect(args.positionals).toEqual(["--not-a-flag"]);
  });
});

describe("flag helpers", () => {
  const args = parseArgs(["-o", "x.ts", "--debug"]);

  test("flagString finds string values", () => {
    expect(flagString(args, "out", "o")).toBe("x.ts");
  });

  test("flagString misses boolean flags", () => {
    expect(flagString(args, "debug")).toBeUndefined();
  });

  test("flagBool sees boolean flags", () => {
    expect(flagBool(args, "debug")).toBeTrue();
    expect(flagBool(args, "skip-build")).toBeFalse();
  });
});
