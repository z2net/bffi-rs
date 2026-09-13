/**
 * The `.bffi/bffi.json` configuration: schema v1, validation, and
 * discovery (walking up from the working directory).
 *
 * The config is the ONLY thing a consumer maintains - everything else
 * (pipeline, generation, binary resolution) is derived from it.
 */
import { BFFI_DIR, CONFIG_FILE } from "./paths.ts";

/** The config schema version this package understands. */
export const CONFIG_VERSION = 1;

/** The Rust crate backing the native module. */
export interface CrateConfig {
  /** Relative path of the crate directory (from the project root).
   * `"."` when the crate IS the project root. */
  dir: string;
  /** Cargo package name (`-p` flag for `cargo build`). */
  name: string;
  /** The cdylib base name (WITHOUT extension/lib prefix). */
  binary: string;
  /** Relative path of the target directory (from the project root).
   * Default: `<dir>/target`. Set it when a cargo WORKSPACE relocates
   * `target/` (e.g. `"../../target"` in a monorepo). */
  targetDir?: string;
}

/** The validated `.bffi/bffi.json` config (all defaults applied). */
export interface BffiConfig {
  /** Schema version (always 1 for now). */
  bffi: number;
  /** Version of the bffi stack (crate/package compatibility). */
  version: string;
  /** The module name (the generated file's header). */
  module: string;
  /** Target OS override; `"auto"` = the running platform. */
  os: "auto" | "win32" | "linux" | "darwin";
  /** Linux libc override for platform-package resolution; `"auto"`
   * assumes glibc. Matters for musl (Alpine) consumers. */
  libc?: "auto" | "glibc" | "musl";
  /** Debug mode: sets `BFFI_DEBUG` + verbose logs. */
  debug: boolean;
  /** The bun executable (spawned subprocesses). */
  bun: string;
  /** The cargo executable (spawned subprocesses). */
  rust: string;
  /** The backing Rust crate. */
  crate: CrateConfig;
  /** Optional CLI version pin (the CLI is a separate project). */
  cli?: { version: string };
  /** Built-in export groups the crate actually expands. Must match
   * the `*_abi!()` macros expanded in the crate: `bun:ffi` throws at
   * dlopen on any declared-but-missing symbol. */
  features?: {
    runtime?: boolean;
    async?: boolean;
    callbacks?: boolean;
    stream?: boolean;
  };
  /** Generation options. */
  generate: { apiGen: boolean; outFile: string };
  /** The working files living inside `.bffi`. */
  files: string[];
  /** Explicit library path override (skips resolution). */
  libraryPath: string | null;
}

/** One validation failure: the JSON path plus the reason. */
export interface ConfigIssue {
  path: string;
  message: string;
}

/** Raised by [`loadConfigFile`]/[`validateConfig`] when the config is
 * malformed; `issues` is never empty. */
export class ConfigValidationError extends Error {
  readonly issues: ConfigIssue[];

  constructor(issues: ConfigIssue[]) {
    super(issues.map((issue) => `${issue.path}: ${issue.message}`).join("\n"));
    this.name = "ConfigValidationError";
    this.issues = issues;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function str(issues: ConfigIssue[], path: string, value: unknown): void {
  if (typeof value !== "string" || value.length === 0) {
    issues.push({ path, message: "expected a non-empty string" });
  }
}

function validate(raw: unknown, issues: ConfigIssue[], path: string): void {
  if (!isRecord(raw)) {
    issues.push({ path, message: "expected a JSON object" });
    return;
  }
  if (raw.bffi !== CONFIG_VERSION) {
    issues.push({
      path: `${path}.bffi`,
      message: `unsupported config version ${JSON.stringify(raw.bffi)} (expected ${CONFIG_VERSION})`,
    });
  }
  str(issues, `${path}.version`, raw.version);
  str(issues, `${path}.module`, raw.module);
  if (raw.os !== undefined && raw.os !== "auto") {
    if (raw.os !== "win32" && raw.os !== "linux" && raw.os !== "darwin") {
      issues.push({
        path: `${path}.os`,
        message: 'expected "auto", "win32", "linux" or "darwin"',
      });
    }
  }
  if (raw.libc !== undefined && raw.libc !== "auto") {
    if (raw.libc !== "glibc" && raw.libc !== "musl") {
      issues.push({
        path: `${path}.libc`,
        message: 'expected "auto", "glibc" or "musl"',
      });
    }
  }
  if (raw.debug !== undefined && typeof raw.debug !== "boolean") {
    issues.push({ path: `${path}.debug`, message: "expected a boolean" });
  }
  if (raw.bun !== undefined) {
    str(issues, `${path}.bun`, raw.bun);
  }
  if (raw.rust !== undefined) {
    str(issues, `${path}.rust`, raw.rust);
  }
  if (!isRecord(raw.crate)) {
    issues.push({ path: `${path}.crate`, message: "expected an object" });
  } else {
    str(issues, `${path}.crate.dir`, raw.crate.dir);
    str(issues, `${path}.crate.name`, raw.crate.name);
    str(issues, `${path}.crate.binary`, raw.crate.binary);
  }
  if (raw.cli !== undefined) {
    if (!isRecord(raw.cli)) {
      issues.push({ path: `${path}.cli`, message: "expected an object" });
    } else {
      str(issues, `${path}.cli.version`, raw.cli.version);
    }
  }
  if (raw.generate !== undefined) {
    if (!isRecord(raw.generate)) {
      issues.push({ path: `${path}.generate`, message: "expected an object" });
    } else {
      if (raw.generate.apiGen !== undefined && typeof raw.generate.apiGen !== "boolean") {
        issues.push({ path: `${path}.generate.apiGen`, message: "expected a boolean" });
      }
      if (raw.generate.outFile !== undefined) {
        str(issues, `${path}.generate.outFile`, raw.generate.outFile);
      }
    }
  }
  if (raw.files !== undefined) {
    if (!Array.isArray(raw.files) || raw.files.some((f) => typeof f !== "string")) {
      issues.push({ path: `${path}.files`, message: "expected an array of strings" });
    }
  }
  if (raw.libraryPath !== undefined && raw.libraryPath !== null) {
    str(issues, `${path}.libraryPath`, raw.libraryPath);
  }
}

/** Validates raw parsed config JSON; throws
 * [`ConfigValidationError`] with every issue found. */
export function validateConfig(raw: unknown): BffiConfig {
  const issues: ConfigIssue[] = [];
  validate(raw, issues, "$");
  if (issues.length > 0) {
    throw new ConfigValidationError(issues);
  }
  const cfg = raw as BffiConfig;
  return {
    ...cfg,
    os: cfg.os ?? "auto",
    libc: cfg.libc ?? "auto",
    debug: cfg.debug ?? false,
    bun: cfg.bun ?? "bun",
    rust: cfg.rust ?? "cargo",
    crate: cfg.crate,
    generate: {
      apiGen: cfg.generate?.apiGen ?? true,
      outFile: cfg.generate?.outFile ?? "api.gen.ts",
    },
    files: cfg.files ?? ["bffi.api.json", cfg.generate?.outFile ?? "api.gen.ts"],
    libraryPath: cfg.libraryPath ?? null,
  };
}

/** Typed authoring helper for `bffi.json` (identity; exists for
 * editor completions). */
export function defineConfig(config: BffiConfig): BffiConfig {
  return config;
}

/** The default location of the config file inside a project. */
export function configPathIn(projectRoot: string): string {
  return `${projectRoot.replace(/\/+$/, "")}/${BFFI_DIR}/${CONFIG_FILE}`;
}

/** Reads and validates `.bffi/bffi.json` under `projectRoot`. */
export async function loadConfigFile(projectRoot: string): Promise<BffiConfig> {
  const path = configPathIn(projectRoot);
  const file = Bun.file(path);
  if (!(await file.exists())) {
    throw new Error(`bffi config not found: ${path}`);
  }
  let raw: unknown;
  try {
    raw = JSON.parse(await file.text());
  } catch (error) {
    throw new ConfigValidationError([
      { path, message: `cannot read or parse JSON: ${String(error)}` },
    ]);
  }
  return validateConfig(raw);
}

/** Walks up from `startDir` looking for `.bffi/bffi.json`; returns
 * the PROJECT ROOT (the directory containing `.bffi`), or undefined. */
export async function findProjectRoot(
  startDir: string = process.cwd(),
): Promise<string | undefined> {
  let dir = startDir.replaceAll("\\", "/").replace(/\/+$/, "");
  while (true) {
    if (await Bun.file(`${dir}/${BFFI_DIR}/${CONFIG_FILE}`).exists()) {
      return dir;
    }
    const parent = dir.slice(0, dir.lastIndexOf("/"));
    if (parent === dir || parent === "") {
      return undefined;
    }
    dir = parent;
  }
}
