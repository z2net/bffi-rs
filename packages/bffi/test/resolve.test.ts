/**
 * Tests for platform-binary resolution: the napi-rs style
 * triple mapping, the artifact-convention file name, and the
 * `<base>-<triple>` -> binary-path resolution (with an injected
 * resolver - no real node_modules involved).
 *
 * Run with `bun test packages/bffi`.
 */
import { afterAll, describe, expect, test } from "bun:test";

import {
  platformTriple,
  resolvePlatformBinary,
  tripleWithLibc,
} from "../src/index.ts";

/** The expected binary path, built with the SAME join the
 * implementation uses (dirname + "/" + file), so the assertions are
 * platform-separator agnostic. */
function expectedBinary(entry: string, file: string): string {
  const normalized = entry.replaceAll("\\", "/");
  return `${normalized.slice(0, normalized.lastIndexOf("/"))}/${file}`;
}

describe("platformTriple", () => {
  test("maps the shipped platforms onto napi-rs triples", () => {
    expect(platformTriple("win32", "x64")).toBe("win32-x64-msvc");
    expect(platformTriple("linux", "x64")).toBe("linux-x64-gnu");
    expect(platformTriple("linux", "arm64")).toBe("linux-arm64-gnu");
    expect(platformTriple("darwin", "arm64")).toBe("darwin-aarch64");
    expect(platformTriple("darwin", "x64")).toBe("darwin-x64");
  });

  test("rejects platforms bffi does not ship", () => {
    expect(() => platformTriple("freebsd", "x64")).toThrow(/unsupported platform/);
  });
});

describe("tripleWithLibc", () => {
  test("musl swaps the gnu suffix on linux triples only", () => {
    expect(tripleWithLibc("linux-x64-gnu", "musl")).toBe("linux-x64-musl");
    expect(tripleWithLibc("linux-arm64-gnu", "musl")).toBe("linux-arm64-musl");
    expect(tripleWithLibc("linux-x64-gnu", "auto")).toBe("linux-x64-gnu");
    expect(tripleWithLibc("linux-x64-gnu", "glibc")).toBe("linux-x64-gnu");
    // Non-linux triples are untouched.
    expect(tripleWithLibc("win32-x64-msvc", "musl")).toBe("win32-x64-msvc");
    expect(tripleWithLibc("darwin-aarch64", "musl")).toBe("darwin-aarch64");
  });
});

describe("resolvePlatformBinary", () => {
  // Expected values are built with the SAME join the implementation
  // uses, so the assertions are platform-separator agnostic.
  test("resolves <base>-<triple> and appends the artifact convention", async () => {
    let seenSpec = "";
    let seenFrom = "";
    const entry = "C:/proj/node_modules/@z2net/mylib-win32-x64-msvc/index.js";
    const path = await resolvePlatformBinary("@z2net/mylib", {
      triple: "win32-x64-msvc",
      binary: "bffi_mylib",
      from: "C:/proj",
      resolveSync: (specifier, from) => {
        seenSpec = specifier;
        seenFrom = from;
        return entry;
      },
    });
    expect(seenSpec).toBe("@z2net/mylib-win32-x64-msvc");
    expect(seenFrom).toBe("C:/proj");
    expect(path).toBe(expectedBinary(entry, "bffi_mylib.dll"));
  });

  test("unix triples carry the lib prefix", async () => {
    const entry = "/pkg/node_modules/@z2net/mylib-linux-x64-gnu/index.js";
    const path = await resolvePlatformBinary("@z2net/mylib", {
      triple: "linux-x64-gnu",
      binary: "bffi_mylib",
      resolveSync: () => entry,
    });
    expect(path).toBe(expectedBinary(entry, "libbffi_mylib.so"));
  });

  test("darwin triples use the dylib extension with the lib prefix", async () => {
    const entry = "/pkg/node_modules/@z2net/mylib-darwin-aarch64/index.js";
    const path = await resolvePlatformBinary("@z2net/mylib", {
      triple: "darwin-aarch64",
      binary: "bffi_mylib",
      resolveSync: () => entry,
    });
    expect(path).toBe(expectedBinary(entry, "libbffi_mylib.dylib"));
  });

  test("a missing platform package produces an actionable error", async () => {
    await expect(
      resolvePlatformBinary("@z2net/mylib", {
        triple: "linux-x64-gnu",
        binary: "bffi_mylib",
        resolveSync: () => {
          throw new Error("MODULE_NOT_FOUND");
        },
      }),
    ).rejects.toThrow(/@z2net\/mylib-linux-x64-gnu is not installed/);
  });
  test("a missing binary option is rejected up front", async () => {
    // The binary check runs BEFORE any resolution, so no default
    // resolver is consulted here. The option is required in the TS
    // types; the runtime guard covers untyped (JS) consumers.
    const untyped = resolvePlatformBinary as unknown as (
      base: string,
      options: Record<string, unknown>,
    ) => Promise<string>;
    await expect(
      untyped("@z2net/mylib", { triple: "win32-x64-msvc" }),
    ).rejects.toThrow(/options\.binary is required/);
  });
});

describe("resolvePlatformBinary integrity", () => {
  // A fake platform package: package.json + a binary file on disk.
  const ROOT = `${import.meta.dir}/tmp-integrity`;
  const PKG_DIR = `${ROOT}/mylib-win32-x64-msvc`;
  const BINARY = `${PKG_DIR}/bffi_mylib.dll`;
  const BYTES = new Uint8Array([9, 8, 7]);
  const DIGEST = (() => {
    const hasher = new Bun.CryptoHasher("sha256");
    hasher.update(BYTES);
    return hasher.digest("hex");
  })();

  const resolve = async (): Promise<string> =>
    resolvePlatformBinary("@z2net/mylib", {
      triple: "win32-x64-msvc",
      binary: "bffi_mylib",
      from: ROOT,
      resolveSync: () => `${PKG_DIR}/index.js`,
    });

  test("a matching integrity digest resolves", async () => {
    await Bun.write(BINARY, BYTES);
    await Bun.write(
      `${PKG_DIR}/package.json`,
      JSON.stringify({ name: "@z2net/mylib-win32-x64-msvc", integrity: `sha256-${DIGEST}` }),
    );
    // The implementation joins with "/", so normalize the fixture
    // path the same way for a separator-agnostic assertion.
    expect(await resolve()).toBe(BINARY.replaceAll("\\", "/"));
  });

  test("a mismatching integrity digest throws", async () => {
    await Bun.write(BINARY, BYTES);
    await Bun.write(
      `${PKG_DIR}/package.json`,
      JSON.stringify({ integrity: `sha256-${"0".repeat(64)}` }),
    );
    await expect(resolve()).rejects.toThrow(
      `integrity mismatch for @z2net/mylib-win32-x64-msvc: expected sha256-${"0".repeat(64)}, got sha256-${DIGEST}`,
    );
  });

  test("a tampered binary throws against a good manifest", async () => {
    await Bun.write(BINARY, new Uint8Array([1]));
    await Bun.write(
      `${PKG_DIR}/package.json`,
      JSON.stringify({ integrity: `sha256-${DIGEST}` }),
    );
    await expect(resolve()).rejects.toThrow(/integrity mismatch for @z2net\/mylib-win32-x64-msvc/);
  });

  test("a package.json without integrity skips the check (legacy)", async () => {
    await Bun.write(`${PKG_DIR}/package.json`, JSON.stringify({ name: "@z2net/mylib-win32-x64-msvc" }));
    await expect(resolve()).resolves.toBeDefined();
  });

  afterAll(async () => {
    const { $ } = await import("bun");
    await $`rm -rf ${ROOT}`;
  });
});
