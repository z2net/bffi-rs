/**
 * Tests for `bffi pack`: assembles a platform package (binary +
 * os/cpu/libc package.json + `{path}` shim) from a fake binary.
 *
 * Run with `bun test packages/bffi-cli`.
 */
import { afterAll, describe, expect, test } from "bun:test";

import { pack } from "#cli/commands/pack.ts";

const ROOT = `${import.meta.dir}/tmp`;
const SRC = `${ROOT}/fake.dll`;

describe("bffi pack", () => {
  test("assembles the platform package with the artifact convention", async () => {
    await Bun.write(SRC, new Uint8Array([1, 2, 3]));
    const code = await pack([
      "--src",
      SRC,
      "--triple",
      "win32-x64-msvc",
      "--name",
      "@z2net/mylib",
      "--binary",
      "bffi_mylib",
      "--out",
      `${ROOT}/platform`,
    ]);
    expect(code).toBe(0);

    const pkgDir = `${ROOT}/platform/mylib-win32-x64-msvc`;

    const pkg = JSON.parse(await Bun.file(`${pkgDir}/package.json`).text());
    expect(pkg.name).toBe("@z2net/mylib-win32-x64-msvc");
    expect(pkg.os).toEqual(["win32"]);
    expect(pkg.cpu).toEqual(["x64"]);
    // The integrity field pins the packed binary to its sha256 digest.
    const hasher = new Bun.CryptoHasher("sha256");
    hasher.update(new Uint8Array([1, 2, 3]));
    expect(pkg.integrity).toBe(`sha256-${hasher.digest("hex")}`);

    const shim = await Bun.file(`${pkgDir}/index.js`).text();
    // The binary follows the artifact convention
    // (`[lib]<binary>.<ext>`), which is what `resolvePlatformBinary`
    // looks up.
    expect(shim).toContain('"/bffi_mylib.dll"');
    // The generated shim must stay free of node: imports (bun-only).
    expect(shim).not.toContain("node:");

    expect(await Bun.file(`${pkgDir}/bffi_mylib.dll`).exists()).toBeTrue();
  });

  test("the binary base name defaults to the package base", async () => {
    const code = await pack([
      "--src",
      SRC,
      "--triple",
      "linux-x64-gnu",
      "--name",
      "@z2net/bffi-native",
      "--out",
      `${ROOT}/platform-default`,
    ]);
    expect(code).toBe(0);
    expect(
      await Bun.file(
        `${ROOT}/platform-default/bffi-native-linux-x64-gnu/libbffi_native.so`,
      ).exists(),
    ).toBeTrue();
  });

  test("an unknown triple fails (exit 2)", async () => {
    const code = await pack([
      "--src",
      SRC,
      "--triple",
      "freebsd-x64",
      "--out",
      `${ROOT}/platform`,
    ]);
    expect(code).toBe(2);
  });

  test("a missing --src is a usage error (exit 1)", async () => {
    expect(await pack(["--triple", "win32-x64-msvc"])).toBe(1);
  });

  afterAll(async () => {
    const { $ } = await import("bun");
    await $`rm -rf ${ROOT}`;
  });
});
