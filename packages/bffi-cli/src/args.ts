/**
 * Manual argv parser (no `node:util`): positional arguments plus
 * `--flag <value>` / `--flag=value` / `-o <value>` flags and boolean
 * flags. `--` stops flag parsing.
 */
export interface ParsedArgs {
  positionals: string[];
  flags: Record<string, string | true>;
}

export function parseArgs(argv: string[]): ParsedArgs {
  const positionals: string[] = [];
  const flags: Record<string, string | true> = {};
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === undefined) {
      break;
    }
    if (arg === "--") {
      for (const rest of argv.slice(i + 1)) {
        positionals.push(rest);
      }
      break;
    }
    if (arg.startsWith("--")) {
      const eq = arg.indexOf("=");
      if (eq > 2) {
        flags[arg.slice(2, eq)] = arg.slice(eq + 1);
        continue;
      }
      const key = arg.slice(2);
      const next = argv[i + 1];
      if (next !== undefined && !next.startsWith("-")) {
        flags[key] = next;
        i++;
      } else {
        flags[key] = true;
      }
      continue;
    }
    if (arg.startsWith("-") && arg.length > 1) {
      const key = arg.slice(1);
      const next = argv[i + 1];
      if (next !== undefined && !next.startsWith("-")) {
        flags[key] = next;
        i++;
      } else {
        flags[key] = true;
      }
      continue;
    }
    positionals.push(arg);
  }
  return { positionals, flags };
}

/** First string value among the given flag names. */
export function flagString(args: ParsedArgs, ...names: string[]): string | undefined {
  for (const name of names) {
    const value = args.flags[name];
    if (typeof value === "string") {
      return value;
    }
  }
  return undefined;
}

/** Whether ANY of the given boolean flags is present. */
export function flagBool(args: ParsedArgs, ...names: string[]): boolean {
  return names.some((name) => args.flags[name] === true);
}
