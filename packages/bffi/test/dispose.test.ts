/**
 * Tests for the per-library disposal registry and the Bun 1.4
 * memory-pressure hook.
 *
 * Run with `bun test packages/bffi`.
 */
import { describe, expect, test } from "bun:test";

import { disposeLib, installMemoryPressureGC, makeLibDisposer, registerDisposer } from "../src/runtime/dispose.ts";
import type { FfiLib } from "../src/runtime/error.ts";

function fakeLib(): FfiLib {
  return {} as FfiLib;
}

describe("dispose registry", () => {
  test("disposeLib runs every registered closer exactly once", () => {
    const lib = fakeLib();
    const ran: string[] = [];
    const set = (() => {
      // The registry is exercised through bindJsCallback in e2e; the
      // unit pins the set contract directly.
      const s1 = registerDisposer(lib, () => ran.push("a"));
      registerDisposer(lib, () => ran.push("b"));
      return s1;
    })();
    expect(set.size).toBe(2);

    makeLibDisposer(lib)[Symbol.dispose]();
    expect(ran.sort()).toEqual(["a", "b"]);

    // Idempotent: the set was cleared.
    makeLibDisposer(lib)[Symbol.dispose]();
    expect(ran).toHaveLength(2);
    disposeLib(lib);
    expect(ran).toHaveLength(2);
  });

  test("a closer removed from the set is not run", () => {
    const lib = fakeLib();
    let ran = 0;
    const { registerDisposer } = require("../src/runtime/dispose.ts") as {
      registerDisposer(lib: FfiLib, closer: () => void): Set<() => void>;
    };
    const closer = (): void => {
      ran += 1;
    };
    const set = registerDisposer(lib, closer);
    set.delete(closer);
    disposeLib(lib);
    expect(ran).toBe(0);
  });

  test("disposeLib on an unknown lib is a no-op", () => {
    expect(() => disposeLib(fakeLib())).not.toThrow();
  });
});

describe("installMemoryPressureGC", () => {
  test("installs once, uninstalls, installs again", () => {
    const uninstall = installMemoryPressureGC();
    const second = installMemoryPressureGC();
    expect(typeof uninstall).toBe("function");
    // The second install is a no-op stub.
    expect(second()).toBeUndefined();

    uninstall();
    // After uninstall a fresh install works again.
    const reinstall = installMemoryPressureGC();
    expect(typeof reinstall).toBe("function");
    reinstall();
  });
});
