/**
 * The runtime Bun version gate (AGENTS.md hard rule: minimum Bun
 * 1.4.2 - raised for the 1.4.1 Windows JIT fix when optimized code
 * passes an ArrayBuffer to a `bun:ffi` pointer argument and the
 * 1.4.2 musl GC and long-running-process JIT crash fixes).
 * `engines` in package.json is advisory only; this module
 * ENFORCES the floor.
 *
 * The comparison is numeric (a `"1.10.0"` must count as newer than
 * `"1.4.2"`, which rules out string comparison) and does NOT use
 * `Bun.semver` - that API postdates older 1.x releases, so the gate
 * must work on the very runtimes it rejects. Prerelease suffixes
 * (`1.4.2-canary.12`) are compared by their leading numeric triple.
 */

/** The minimum Bun version this framework supports. */
export const MIN_BUN_VERSION = "1.4.2";

/** Parses the leading numeric triple of a version string. */
function parseTuple(version: string): [number, number, number] | undefined {
  const match = /^(\d+)\.(\d+)(?:\.(\d+))?/.exec(version.trim());
  if (!match) {
    return undefined;
  }
  return [
    Number(match[1]),
    Number(match[2]),
    match[3] === undefined ? 0 : Number(match[3]),
  ];
}

/**
 * Whether `version` satisfies the minimum. `undefined` (no Bun
 * runtime) and unparsable strings are `false`.
 */
export function bunVersionSatisfies(
  version: string | undefined,
  min: string = MIN_BUN_VERSION,
): boolean {
  const current = version === undefined ? undefined : parseTuple(version);
  const required = parseTuple(min);
  if (current === undefined || required === undefined) {
    return false;
  }
  for (let i = 0; i < 3; i++) {
    const a = current[i] ?? 0;
    const b = required[i] ?? 0;
    if (a !== b) {
      return a > b;
    }
  }
  return true;
}

/**
 * The failure message for a version gate violation (shared by the
 * library throw and the CLI stderr line).
 */
export function bunVersionProblem(
  version: string | undefined,
  min: string = MIN_BUN_VERSION,
): string | undefined {
  if (bunVersionSatisfies(version, min)) {
    return undefined;
  }
  if (version === undefined) {
    return `requires Bun >= ${min}; no Bun runtime detected`;
  }
  return `requires Bun >= ${min}; found ${version}. Upgrade Bun: https://bun.sh`;
}
/**
 * Throws when the running Bun is older than [`MIN_BUN_VERSION`].
 * Called at the public module entry so consumers fail fast with a
 * clear message instead of obscure `bun:ffi` errors later.
 */
export function assertBunVersion(version: string = Bun.version): void {
  const problem = bunVersionProblem(version);
  if (problem !== undefined) {
    throw new Error(`@z2net/bffi ${problem}`);
  }
}
