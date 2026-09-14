/**
 * Tests for the full-pipeline trust gate: `bffi()` only dlopens a
 * raw `libraryPath` when the call passes `trust: "explicit"` -
 * platform-package resolution is the default. The gate runs before
 * ANY pipeline work (config discovery included), so the tests need
 * no mocks beyond a config path that must not be reached.
 *
 * Run with `bun test packages/bffi`.
 */
import { describe, expect, test } from "bun:test";

import { bffi } from "../src/index.ts";

const MISSING = "Z:/bffi-pipeline-test-missing/.bffi/bffi.json";

describe("bffi() trust gate", () => {
  test("libraryPath without trust: 'explicit' throws before any pipeline work", async () => {
    expect(
      bffi({ libraryPath: "Z:/bffi-pipeline-test-missing/fake.dll", config: MISSING }),
    ).rejects.toThrow(
      'libraryPath is an explicit trust decision - pass trust: "explicit" to load ' +
        "a specific binary path (default: platform-package resolution)",
    );
  });

  test("trust: 'explicit' passes the gate and proceeds to the pipeline", async () => {
    // The gate passes; the pipeline then fails on the missing config -
    // a DIFFERENT error, proving the gate is not what threw.
    expect(
      bffi({
        libraryPath: "Z:/bffi-pipeline-test-missing/fake.dll",
        trust: "explicit",
        config: MISSING,
      }),
    ).rejects.toThrow(/bffi config not found/);
  });

  test("no libraryPath: the gate does not apply", async () => {
    expect(bffi({ config: MISSING })).rejects.toThrow(/bffi config not found/);
  });
});
