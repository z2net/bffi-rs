/** `bffi pack --src <binary> --triple <t> [--name <base>] [--binary <b>] [--out <dir>]`:
 * assembles one platform npm package (napi-rs style) from a built
 * cdylib - the binary (renamed to the artifact convention
 * `[lib]<binary>.<ext>` that `resolvePlatformBinary` looks up), a
 * package.json carrying os/cpu/libc, and an `index.js` entry shim.
 *
 * Bun-only: all file operations go through `Bun.file`/`Bun.write`
 * (`Bun.write` copies from a `BunFile` and creates the parent
 * directories itself - no `node:` module imports). */
import { joinOut, platformTriple } from "@z2net/bffi";
import { flagString, parseArgs } from "../args.ts";
import { EXIT, writeErr, writeOut } from "../output.ts";

export const usage =
  `bffi pack --src <binary> --triple <t> [--name <base>] [--binary <b>] [--out <dir>]`;

/** triple -> npm os/cpu/libc fields. Extend the map when you add
 * targets. The binary file name follows the artifact convention:
 * `<binary>.dll` on Windows, `lib<binary>.so|.dylib` elsewhere. */
const PLATFORMS: Record<
  string,
  { os: string; cpu: string; libc?: string; ext: string; prefix: string }
> = {
  "win32-x64-msvc": { os: "win32", cpu: "x64", ext: "dll", prefix: "" },
  "win32-arm64-msvc": { os: "win32", cpu: "arm64", ext: "dll", prefix: "" },
  "linux-x64-gnu": { os: "linux", cpu: "x64", libc: "glibc", ext: "so", prefix: "lib" },
  "linux-arm64-gnu": { os: "linux", cpu: "arm64", libc: "glibc", ext: "so", prefix: "lib" },
  "linux-x64-musl": { os: "linux", cpu: "x64", libc: "musl", ext: "so", prefix: "lib" },
  "linux-arm64-musl": { os: "linux", cpu: "arm64", libc: "musl", ext: "so", prefix: "lib" },
  "darwin-aarch64": { os: "darwin", cpu: "arm64", ext: "dylib", prefix: "lib" },
  "darwin-x64": { os: "darwin", cpu: "x64", ext: "dylib", prefix: "lib" },
};

export async function pack(argv: string[]): Promise<number> {
  const args = parseArgs(argv);
  const src = flagString(args, "src");
  const triple = flagString(args, "triple") ?? platformTriple();
  const base = flagString(args, "name") ?? "@z2net/bffi-native";
  const outDir = flagString(args, "out") ?? "platform";

  if (src === undefined) {
    writeErr(usage);
    return EXIT.usage;
  }
  const platform = PLATFORMS[triple];
  if (platform === undefined) {
    writeErr(
      `pack: unknown triple ${triple} (shipped: ${Object.keys(PLATFORMS).join(", ")})`,
    );
    return EXIT.fail;
  }

  const main = JSON.parse(
    await Bun.file(joinOut(process.cwd(), "package.json")).text(),
  ) as { name: string; version: string; license?: string };
  const parts = base.split("/");
  const scopeless = parts.length > 1 ? parts[parts.length - 1] ?? base : base;
  // The cdylib base name (`bffi_mylib`): defaults to the package base
  // with dashes replaced, matching the Rust `binary` config field.
  const binary =
    flagString(args, "binary") ?? scopeless.replaceAll("-", "_");
  const file = `${platform.prefix}${binary}.${platform.ext}`;
  const pkgName = `${base}-${triple}`;
  const pkgDir = joinOut(process.cwd(), outDir, `${scopeless}-${triple}`);

  // The binary is read once and hashed BEFORE the package.json is
  // written: the `integrity` field pins the packed artifact to its
  // sha256 digest (`resolvePlatformBinary` verifies it at load).
  const bytes = new Uint8Array(await Bun.file(src).arrayBuffer());
  const hasher = new Bun.CryptoHasher("sha256");
  hasher.update(bytes);
  const integrity = `sha256-${hasher.digest("hex")}`;

  const pkg = {
    name: pkgName,
    version: main.version,
    description: `${base} native binary (${triple})`,
    license: main.license ?? "MIT",
    main: "index.js",
    os: [platform.os],
    cpu: [platform.cpu],
    ...(platform.libc === undefined ? {} : { libc: [platform.libc] }),
    integrity,
  };
  // The package.json write creates the package directory tree.
  await Bun.write(
    joinOut(pkgDir, "package.json"),
    `${JSON.stringify(pkg, null, 2)}\n`,
  );
  // Copy (never move) the binary under the artifact-convention name.
  await Bun.write(joinOut(pkgDir, file), bytes);

  // The CJS entry shim: the opaque `{ path }` contract. Plain string
  // concat - forward slashes work in every Bun file API on all
  // platforms, so no path module is needed here either.
  const shim = `"use strict";
module.exports = { path: __dirname + ${JSON.stringify(`/${file}`)} };
`;
  await Bun.write(joinOut(pkgDir, "index.js"), shim);

  writeOut(`packed ${pkgName} -> ${pkgDir} (${file})`);
  return EXIT.ok;
}

