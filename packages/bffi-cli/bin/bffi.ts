#!/usr/bin/env bun
/**
 * The `bffi` CLI - a thin wrapper over `@z2net/bffi` (all pipeline
 * logic lives there). Commands: init, build, check, codegen, doctor,
 * pack, fetch.
 *
 * Exit codes: 0 = ok, 1 = usage error, 2 = input/environment failure.
 */
import { bunVersionProblem } from "@z2net/bffi";
import { build } from "#cli/commands/build.ts";
import { check } from "#cli/commands/check.ts";
import { codegen } from "#cli/commands/codegen.ts";
import { doctor } from "#cli/commands/doctor.ts";
import { fetchCmd } from "#cli/commands/fetch.ts";
import { init } from "#cli/commands/init.ts";
import { pack } from "#cli/commands/pack.ts";
import { USAGE } from "#cli/usage.ts";
import { EXIT, writeErr } from "#cli/output.ts";

type Command = (argv: string[]) => Promise<number>;

const COMMANDS: Record<string, Command> = {
  init,
  build,
  check,
  codegen,
  doctor,
  pack,
  fetch: fetchCmd,
};

/** The process entry point; resolves to the exit code. */
export async function main(argv: string[]): Promise<number> {
  // Environment gate first (exit 2: the runtime itself is too old).
  const versionProblem = bunVersionProblem(Bun.version);
  if (versionProblem !== undefined) {
    writeErr(`bffi ${versionProblem}`);
    return EXIT.fail;
  }

  const [command, ...rest] = argv;
  if (command === undefined || command === "help" || command === "--help" || command === "-h") {
    writeErr(USAGE);
    return command === undefined ? EXIT.usage : EXIT.ok;
  }

  const handler = COMMANDS[command];
  if (handler === undefined) {
    writeErr(`bffi: unknown command "${command}"\n\n${USAGE}`);
    return EXIT.usage;
  }

  try {
    return await handler(rest);
  } catch (error) {
    writeErr(`bffi ${command}: ${String(error)}`);
    return EXIT.fail;
  }
}

// The direct-run entry point (tests import `main` instead).
if (import.meta.main) {
  void main(process.argv.slice(2)).then((code) => (process.exitCode = code));
}
