/**
 * Platform-binary resolution for platform-package distribution
 * (napi-rs style): the main package declares every platform package
 * in `optionalDependencies`; npm/bun installs ONLY the one matching
 * the running platform (`os`/`cpu`/`libc` fields), and this module
 * turns that package into the dlopen path.
 *
 * Triple naming follows the napi-rs convention
 * (`<base>-win32-x64-msvc`, `<base>-linux-x64-gnu`,
 * `<base>-darwin-aarch64`, ...). The binary inside the platform
 * package follows the artifact convention
 * `[lib]<binary>.<ext>` (`bffi_mylib.dll`, `libbffi_mylib.so`, ...),
 * so no package code has to execute - resolution is pure lookup.
 *
 * Bun-only: module resolution goes through `Bun.resolveSync`
 * (a runtime built-in); path handling is pure string logic. The
 * integrity verification reads the platform package's manifest and
 * binary through `Bun.file` - `resolvePlatformBinary` is therefore
 * async (the integrity digest must be verified before the dlopen
 * path is handed out, and Bun's native file readers are async).
 */

/** The napi-rs style triple of the RUNNING platform. Throws for
 * platforms bffi does not ship. */
export function platformTriple(
  platform: string = process.platform,
  arch: string = process.arch,
): string {
  const key = `${platform}-${arch}`;
  switch (key) {
    case "win32-x64":
      return "win32-x64-msvc";
    case "win32-arm64":
      return "win32-arm64-msvc";
    case "linux-x64":
      return "linux-x64-gnu";
    case "linux-arm64":
      return "linux-arm64-gnu";
    case "darwin-arm64":
      return "darwin-aarch64";
    case "darwin-x64":
      return "darwin-x64";
    default:
      throw new Error(
        `unsupported platform for bffi native packages: ${key} ` +
          `(Bun itself ships 64-bit builds only: win32-x64/arm64, linux-x64/arm64, ` +
          `darwin-x64/arm64 - 32-bit systems are not supported)`,
      );
  }
}

/** The dlopen artifact extension of a triple. */
function artifactExt(triple: string): { ext: string; prefix: string } {
  if (triple.startsWith("win32")) {
    return { ext: "dll", prefix: "" };
  }
  if (triple.startsWith("darwin")) {
    return { ext: "dylib", prefix: "lib" };
  }
  return { ext: "so", prefix: "lib" };
}

/** Options of [`resolvePlatformBinary`]. */
export interface ResolveOptions {
  /** Override the detected platform triple. */
  triple?: string;
  /** Linux libc override: `"musl"` turns the default `linux-*-gnu`
   * triple into `linux-*-musl` (Alpine). Default: gnu. */
  libc?: "auto" | "glibc" | "musl";
  /** The cdylib base name (`bffi_mylib` - WITHOUT extension/lib
   * prefix). Required. */
  binary: string;
  /** Resolution base directory (default: `process.cwd()`). */
  from?: string;
  /** Module resolver; defaults to `Bun.resolveSync`. Injectable for
   * tests. */
  resolveSync?: (specifier: string, from: string) => string;
}

/** Applies the libc override to a base triple: `"musl"` swaps the
 * gnu suffix for musl on linux triples. */
export function tripleWithLibc(
  triple: string,
  libc: "auto" | "glibc" | "musl",
): string {
  return libc === "musl" && triple.startsWith("linux-")
    ? triple.replace("-gnu", "-musl")
    : triple;
}

/** The directory part of a resolved module path: everything before
 * the last separator (both `/` and `\` are accepted; the result uses
 * `/`, which every Bun file API accepts on all platforms). */
function dirnameOf(path: string): string {
  const normalized = path.replaceAll("\\", "/");
  const cut = normalized.lastIndexOf("/");
  if (cut <= 0) {
    return normalized;
  }
  return normalized.slice(0, cut);
}

/**
 * Verifies the `integrity` field (`sha256-<hex>`, written by
 * `bffi pack`) of the platform package's package.json against the
 * actual digest of the binary file. A package without the field (or
 * without a readable manifest) skips the check - legacy packages
 * stay loadable.
 */
async function assertIntegrity(pkgName: string, pkgDir: string, binaryPath: string): Promise<void> {
  let expected: unknown;
  try {
    const manifest = (await Bun.file(`${pkgDir}/package.json`).json()) as {
      integrity?: unknown;
    };
    expected = manifest.integrity;
  } catch {
    return; // no readable manifest: nothing to verify
  }
  if (typeof expected !== "string" || !expected.startsWith("sha256-")) {
    return;
  }
  const hasher = new Bun.CryptoHasher("sha256");
  hasher.update(await Bun.file(binaryPath).bytes());
  const got = `sha256-${hasher.digest("hex")}`;
  if (got !== expected) {
    throw new Error(`integrity mismatch for ${pkgName}: expected ${expected}, got ${got}`);
  }
}

/**
 * Resolves the absolute path of the native binary inside the
 * platform package `<base>-<triple>` of `base` (e.g.
 * `@z2net/mylib` -> `@z2net/mylib-win32-x64-msvc`).
 *
 * When the platform package carries an `integrity` digest (written
 * by `bffi pack`), the binary file is hashed and verified before the
 * path is returned.
 *
 * Throws a clear error when the platform package is not installed
 * (optional dependencies can be skipped by package managers).
 */
export async function resolvePlatformBinary(
  base: string,
  options: ResolveOptions,
): Promise<string> {
  const triple = options.triple ?? tripleWithLibc(platformTriple(), options.libc ?? "auto");
  if (options.binary === undefined || options.binary.length === 0) {
    throw new Error(
      `resolvePlatformBinary(${base}): options.binary is required ` +
        `(the cdylib base name, e.g. "bffi_mylib")`,
    );
  }
  const packageName = `${base}-${triple}`;
  const from = options.from ?? process.cwd();
  const resolveSync =
    options.resolveSync ?? ((specifier: string, fromDir: string) => Bun.resolveSync(specifier, fromDir));
  let entry: string;
  try {
    entry = resolveSync(packageName, from);
  } catch (error) {
    throw new Error(
      `native package ${packageName} is not installed or failed to resolve ` +
        `(${String(error)}). Install it explicitly or pass an absolute ` +
        `library path instead.`,
    );
  }
  const { ext, prefix } = artifactExt(triple);
  const pkgDir = dirnameOf(entry);
  const binaryPath = `${pkgDir}/${prefix}${options.binary}.${ext}`;
  await assertIntegrity(packageName, pkgDir, binaryPath);
  return binaryPath;
}
