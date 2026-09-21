/**
 * Tests for `bffi init`: scaffolds .bffi/bffi.json + the crate
 * skeleton into a temp dir; refuses an existing config.
 *
 * Run with `bun test packages/bffi-cli`.
 */
import { afterAll, describe, expect, test } from "bun:test";

import { init } from "#cli/commands/init.ts";

const ROOT = `${import.meta.dir}/tmp`;

describe("bffi init", () => {
  test("scaffolds the config and the crate skeleton", async () => {
    const code = await init([
      "--root",
      ROOT,
      "--module",
      "demo",
      "--crate-dir",
      "crate",
      "--crate-name",
      "bffi-demo",
      "--binary",
      "bffi_demo",
    ]);
    expect(code).toBe(0);

    const config = JSON.parse(
      await Bun.file(`${ROOT}/.bffi/bffi.json`).text(),
    );
    expect(config.bffi).toBe(1);
    expect(config.module).toBe("demo");
    expect(config.crate).toEqual({
      dir: "crate",
      name: "bffi-demo",
      binary: "bffi_demo",
    });

    // The crate skeleton: entry, aggregation, emit-json, cargo file.
    expect(await Bun.file(`${ROOT}/crate/Cargo.toml`).exists()).toBeTrue();
    expect(await Bun.file(`${ROOT}/crate/src/lib.rs`).exists()).toBeTrue();
    expect(await Bun.file(`${ROOT}/crate/src/module_def.rs`).exists()).toBeTrue();
    expect(await Bun.file(`${ROOT}/crate/src/bin/emit_json.rs`).exists()).toBeTrue();
    const lib = await Bun.file(`${ROOT}/crate/src/lib.rs`).text();
    expect(lib).toContain("bffi::bffi_runtime_abi!()");
    expect(lib).toContain("#[bffi::bffi]");
  });

  test("refuses to overwrite an existing config", async () => {
    const code = await init(["--root", ROOT]);
    expect(code).toBe(2);
  });

  afterAll(async () => {
    const { $ } = await import("bun");
    await $`rm -rf ${ROOT}`;
  });
});
