/** `bffi codegen <input.json> -o <out.ts> [--runtime <module>]`:
 * renders a typed TS module from a loader JSON. Deterministic - the
 * same input yields a byte-identical file (safe to commit). */
import { renderModule, SchemaValidationError } from "@z2net/bffi";
import { flagString, parseArgs } from "#cli/args.ts";
import { EXIT, writeErr } from "#cli/output.ts";

export const usage = `bffi codegen <input.json> -o <out.ts> [--runtime <module>]`;

export async function codegen(argv: string[]): Promise<number> {
  const args = parseArgs(argv);
  const input = args.positionals[0];
  const out = flagString(args, "out", "o");
  const runtime = flagString(args, "runtime") ?? "@z2net/bffi";
  if (input === undefined || out === undefined) {
    writeErr(usage);
    return EXIT.usage;
  }

  let raw: unknown;
  try {
    raw = JSON.parse(await Bun.file(input).text());
  } catch (error) {
    writeErr(`${input}: cannot read or parse JSON: ${String(error)}`);
    return EXIT.fail;
  }

  try {
    await Bun.write(out, renderModule(raw, runtime));
    return EXIT.ok;
  } catch (error) {
    if (error instanceof SchemaValidationError) {
      for (const issue of error.issues) {
        writeErr(`${input} ${issue.path}: ${issue.message}`);
      }
      return EXIT.fail;
    }
    throw error;
  }
}
