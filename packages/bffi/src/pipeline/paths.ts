/**
 * `.bffi` working-directory constants plus the path helpers.
 *
 * Path joining is the pure-string [`joinOut`] - no `node:` module
 * imports anywhere in the package.
 */

/** The `.bffi` working directory name (relative). */
export const BFFI_DIR = ".bffi";

/** The config file name inside `.bffi`. */
export const CONFIG_FILE = "bffi.json";

/** The project root of an explicit config file path: the directory
 * HOLDING the `.bffi/` segment, or - when the path carries no
 * `.bffi/` segment - the config file's own directory. The single
 * implementation shared by the pipeline and the CLI commands (the
 * duplicated inline versions drifted: one sliced `slice(0, -1)` on a
 * path without the segment). */
export function rootFromConfigPath(configPath: string): string {
  const normalized = configPath.replaceAll("\\", "/");
  const cut = normalized.lastIndexOf(`/${BFFI_DIR}/`);
  if (cut > 0) {
    return normalized.slice(0, cut);
  }
  return normalized.slice(0, normalized.lastIndexOf("/"));
}

/** Joins `root` with every part of `parts` in order. Each part is
 * normalized to forward slashes; an ABSOLUTE part (`/x` or `C:/x`)
 * wins over everything joined before it (resolve-style), and empty
 * or `.` parts are skipped. */
export function joinOut(root: string, ...parts: string[]): string {
  let joined = root;
  for (const part of parts) {
    const normalized = part.replaceAll("\\", "/");
    if (normalized.length === 0 || normalized === "." || normalized === "./") {
      continue;
    }
    if (normalized.startsWith("/") || /^[A-Za-z]:\//.test(normalized)) {
      joined = normalized;
      continue;
    }
    joined = `${joined.replace(/\/+$/, "")}/${normalized.replace(/^\.\/+/, "").replace(/^\/+/, "")}`;
  }
  return joined;
}
