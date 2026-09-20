#!/usr/bin/env bun
/**
 * Builds standalone `bffi` executables: `bun build --compile --bytecode`
 * over packages/bffi-cli/bin/bffi.ts, one per bun compile target.
 *
 * Usage:
 *   bun scripts/compile-cli.ts              host target only (default)
 *   bun scripts/compile-cli.ts --host-only  same, explicit
 *   bun scripts/compile-cli.ts --all        every shipped target
 *
 * Output: target/compiled/bffi-<triple>[.exe] (gitignored). Requires
 * Bun >= 1.4.2: bytecode cross-compilation needs 1.4.1+ and the repo
 * floor is 1.4.2 - older local runtimes skip with a message, CI
 * (release-compiled, bun 1.4.2) is the real gate.
 */

interface CompiledTarget {
  /** File-name triple: bffi-<triple>[.exe] */
  triple: string;
  /** `bun build --compile --target=<target>` value. */
  target: string;
  /** Windows executables carry the .exe suffix. */
  exe: boolean;
}

const TARGETS: readonly CompiledTarget[] = [
  { triple: "windows-x64", target: "bun-windows-x64", exe: true },
  { triple: "windows-arm64", target: "bun-windows-arm64", exe: true },
  { triple: "linux-x64", target: "bun-linux-x64", exe: false },
  { triple: "linux-x64-musl", target: "bun-linux-x64-musl", exe: false },
  { triple: "linux-arm64", target: "bun-linux-arm64", exe: false },
  { triple: "linux-arm64-musl", target: "bun-linux-arm64-musl", exe: false },
  { triple: "darwin-x64", target: "bun-darwin-x64", exe: false },
  { triple: "darwin-aarch64", target: "bun-darwin-aarch64", exe: false },
];

const ROOT: string = import.meta.dir.replace(/[/\\]scripts$/, "");
const ENTRY: string = `${ROOT}/packages/bffi-cli/bin/bffi.ts`;
const OUT_DIR: string = `${ROOT}/target/compiled`;

function out(line: string): void {
  process.stdout.write(`${line}\n`);
}

function err(line: string): void {
  process.stderr.write(`${line}\n`);
}

/** Numeric >= 1.4.2 comparison (the CLI's own gate, mirrored here). */
function bunMeetsFloor(version: string): boolean {
  const [major, minor, patch] = version.split(".").map((p) => Number.parseInt(p, 10));
  if (major === undefined || minor === undefined || Number.isNaN(major) || Number.isNaN(minor)) {
    return false;
  }
  if (major !== 1) return major > 1;
  if (minor !== 4) return minor > 4;
  return (patch ?? 0) >= 2;
}

/** glibc vs musl on a linux host: ldd's banner is the portable tell. */
function isMuslHost(): boolean {
  const probe = Bun.spawnSync({
    cmd: ["sh", "-c", "ldd --version 2>&1 || true"],
    stdout: "pipe",
    stderr: "pipe",
  });
  return `${probe.stdout}${probe.stderr}`.includes("musl");
}

/** The triple of the machine running this script, in TARGETS terms. */
function hostTriple(): string {
  const platform = process.platform === "win32" ? "windows" : process.platform;
  const arch = process.arch === "x64" ? "x64" : "arm64";
  if (process.platform !== "linux") return `${platform}-${arch}`;
  return `${platform}-${arch}${isMuslHost() ? "-musl" : ""}`;
}

function rel(file: string): string {
  return file.startsWith(`${ROOT}/`) || file.startsWith(`${ROOT}\\`)
    ? file.slice(ROOT.length + 1)
    : file;
}

async function main(): Promise<number> {
  const args = process.argv.slice(2);
  const all = args.includes("--all");
  const bad = args.filter((a) => a !== "--all" && a !== "--host-only");
  if (bad.length > 0) {
    err(`compile:cli: unknown argument(s) ${bad.join(", ")}; usage: bun scripts/compile-cli.ts [--host-only] [--all]`);
    return 1;
  }

  if (!bunMeetsFloor(Bun.version)) {
    out(`SKIP compile:cli: bun ${Bun.version} is below 1.4.2 - bytecode cross-compilation needs 1.4.1+ and the repo floor is 1.4.2. Nothing was built; CI (release-compiled, bun 1.4.2) is the real gate.`);
    return 0;
  }

  const host = hostTriple();
  const selected = all ? TARGETS : TARGETS.filter((t) => t.triple === host);
  if (selected.length === 0) {
    err(`compile:cli: unsupported host "${host}"; supported: ${TARGETS.map((t) => t.triple).join(", ")}`);
    return 1;
  }

  out(`compile:cli: ${all ? "all targets" : `host target (${host})`}, bun ${Bun.version}`);

  interface Row {
    target: string;
    ok: boolean;
    path: string;
    log: string;
  }
  const rows: Row[] = [];
  for (const t of selected) {
    const file = `${OUT_DIR}/bffi-${t.triple}${t.exe ? ".exe" : ""}`;
    const proc = Bun.spawnSync({
      cmd: [
        process.execPath,
        "build",
        "--compile",
        "--bytecode",
        `--target=${t.target}`,
        ENTRY,
        `--outfile=${file}`,
      ],
      cwd: ROOT,
      stdout: "pipe",
      stderr: "pipe",
    });
    const log = `${proc.stdout}${proc.stderr}`.trim();
    const ok = proc.exitCode === 0 && (await Bun.file(file).exists());
    rows.push({ target: t.target, ok, path: ok ? rel(file) : "(not built)", log });
    if (ok) {
      out(`  built ${t.target} -> ${rel(file)}`);
    } else {
      err(`compile:cli: ${t.target} failed (exit ${proc.exitCode})\n${log}`);
    }
  }

  out("");
  out("target                    status  path");
  for (const r of rows) {
    out(`  ${r.target.padEnd(24)}${r.ok ? "ok" : "FAIL"}    ${r.path}`);
  }
  const failed = rows.filter((r) => !r.ok).length;
  out(failed === 0 ? `done: ${rows.length}/${rows.length} built` : `done: ${failed}/${rows.length} FAILED`);
  return failed === 0 ? 0 : 1;
}

void main().then((code) => (process.exitCode = code));
