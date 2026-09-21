/** `bffi check [--config <p>] [--root <d>]`: validates the project
 * WITHOUT building - config, loader JSON, generated file, artifact
 * presence. Exit 0 when everything passes, 2 otherwise. */
import { findProjectRoot, rootFromConfigPath } from "@z2net/bffi";
import { flagString, parseArgs } from "#cli/args.ts";
import { runProjectChecks, type CheckResult } from "#cli/checks.ts";
import { EXIT, writeOut } from "#cli/output.ts";

export const usage = `bffi check [--config <p>] [--root <d>]`;

/** Prints one line per check: `ok   name - detail` / `FAIL name - detail`. */
export function printResults(results: CheckResult[]): void {
  for (const r of results) {
    const mark = r.ok ? "ok  " : "FAIL";
    const detail = r.detail.length > 0 ? ` - ${r.detail}` : "";
    writeOut(`${mark}  ${r.name}${detail}`);
  }
}

export async function check(argv: string[]): Promise<number> {
  const args = parseArgs(argv);
  const config = flagString(args, "config");
  const rootFlag = flagString(args, "root");

  const resolved =
    config !== undefined
      ? rootFromConfigPath(config)
      : (await findProjectRoot(rootFlag)) ?? "";
  if (resolved.length === 0) {
    writeOut("FAIL  config (.bffi/bffi.json) - not found (walked up from the working directory)");
    return EXIT.fail;
  }

  const run = await runProjectChecks(resolved);
  printResults(run.results);
  return run.allOk ? EXIT.ok : EXIT.fail;
}
