/**
 * Shared diagnostics for the `check` and `doctor` commands: each
 * check returns a uniform result the commands aggregate and print.
 *
 * `check` = project validation only (config, loader JSON, generated
 * file, artifact). `doctor` = that PLUS the environment (bun
 * runtime, cargo executable).
 */
import {
  bunVersionProblem,
  joinOut,
  localArtifactPath,
  loadConfigFile,
  validateModule,
  type BffiConfig,
} from "@z2net/bffi";

/** The outcome of one diagnostic. */
export interface CheckResult {
  /** Human-readable check name. */
  name: string;
  ok: boolean;
  /** Extra context (versions, paths, failure reasons). */
  detail: string;
}

/** The aggregate outcome of a check run. */
export interface CheckRun {
  results: CheckResult[];
  /** Whether every check passed. */
  allOk: boolean;
}

function result(name: string, ok: boolean, detail = ""): CheckResult {
  return { name, ok, detail };
}

/** Bun runtime satisfies the 1.4.2 floor. */
export function checkBunRuntime(): CheckResult {
  const problem = bunVersionProblem(Bun.version);
  return problem === undefined
    ? result(`bun runtime (${String(Bun.version)})`, true, ">= 1.4.2")
    : result("bun runtime", false, problem);
}

/** The configured cargo executable responds to `--version`. Uses
 * `Bun.which` first (a missing binary must fail fast, not hang) and
 * caps the spawn with a timeout. */
export function checkCargo(rust: string): CheckResult {
  if (Bun.which(rust) === null) {
    return result(`cargo (${rust})`, false, "not found in PATH");
  }
  try {
    const proc = Bun.spawnSync([rust, "--version"], {
      stdout: "pipe",
      stderr: "pipe",
      timeout: 15000,
    });
    const out = proc.stdout.toString().trim();
    if (proc.exitCode === null) {
      return result(`cargo (${rust})`, false, "spawn timed out");
    }
    return proc.exitCode === 0
      ? result(`cargo (${rust})`, true, out)
      : result(`cargo (${rust})`, false, `exit code ${String(proc.exitCode)}`);
  } catch (error) {
    return result(`cargo (${rust})`, false, String(error));
  }
}

/** The config at `<root>/.bffi/bffi.json` loads and validates.
 * Returns the config on success for follow-up checks. */
export async function checkConfig(
  root: string,
): Promise<CheckResult & { config?: BffiConfig }> {
  try {
    const config = await loadConfigFile(root);
    return {
      ...result(
        "config (.bffi/bffi.json)",
        true,
        `module "${config.module}", v${config.version}`,
      ),
      config,
    };
  } catch (error) {
    return result("config (.bffi/bffi.json)", false, String(error).split("\n")[0] ?? "");
  }
}

/** The loader JSON working file exists and passes schema validation. */
export async function checkLoaderJson(
  root: string,
  config: BffiConfig,
): Promise<CheckResult> {
  const path = joinOut(root, ".bffi", config.files[0] ?? "bffi.api.json");
  const file = Bun.file(path);
  if (!(await file.exists())) {
    return result("loader JSON", false, `missing: ${path}`);
  }
  try {
    const module = validateModule(JSON.parse(await file.text()));
    return result("loader JSON", true, `module "${module.module}"`);
  } catch (error) {
    return result("loader JSON", false, String(error).split("\n")[0] ?? "");
  }
}

/** The generated api.gen.ts exists (when generation is enabled). */
export async function checkGenerated(
  root: string,
  config: BffiConfig,
): Promise<CheckResult> {
  if (config.generate.apiGen === false) {
    return result("api.gen.ts", true, "generation disabled in config");
  }
  const path = joinOut(root, ".bffi", config.generate.outFile);
  return (await Bun.file(path).exists())
    ? result("api.gen.ts", true, path)
    : result("api.gen.ts", false, `missing: ${path} (run bffi build or bffiGenerate)`);
}

/** The native artifact exists at the resolved local path. */
export async function checkArtifact(
  root: string,
  config: BffiConfig,
): Promise<CheckResult> {
  if (config.libraryPath !== null) {
    return (await Bun.file(config.libraryPath).exists())
      ? result("artifact", true, config.libraryPath)
      : result("artifact", false, `missing: ${config.libraryPath}`);
  }
  const path = localArtifactPath(config, root);
  return (await Bun.file(path).exists())
    ? result("artifact", true, path)
    : result(
        "artifact",
        false,
        `missing: ${path} (run cargo build --release -p ${config.crate.name})`,
      );
}

/** Runs config + derived-file checks (the `check` command core). */
export async function runProjectChecks(root: string): Promise<CheckRun> {
  const results: CheckResult[] = [];
  const configCheck = await checkConfig(root);
  results.push(configCheck);
  if (configCheck.ok && configCheck.config !== undefined) {
    const config = configCheck.config;
    results.push(await checkLoaderJson(root, config));
    results.push(await checkGenerated(root, config));
    results.push(await checkArtifact(root, config));
  }
  return { results, allOk: results.every((r) => r.ok) };
}

/** Runs project checks PLUS the environment checks (the `doctor`
 * command core). */
export async function runDoctorChecks(root: string): Promise<CheckRun> {
  const results: CheckResult[] = [checkBunRuntime()];

  const configCheck = await checkConfig(root);
  results.push(configCheck);
  const config = configCheck.config;
  if (config !== undefined) {
    results.push(checkCargo(config.rust));
    results.push(await checkLoaderJson(root, config));
    results.push(await checkGenerated(root, config));
    results.push(await checkArtifact(root, config));
  }

  return { results, allOk: results.every((r) => r.ok) };
}
