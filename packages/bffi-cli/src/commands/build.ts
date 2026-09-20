/** `bffi build [--config <p>] [--skip-build] [--debug]`: runs the
 * FULL pipeline (cargo build -> loader JSON -> api.gen -> binary
 * resolution -> dlopen probe) and prints the resolved artifact path. */
import { bffi, localArtifactPath, loadConfigFile, rootFromConfigPath } from "@z2net/bffi";
import { flagBool, flagString, parseArgs } from "../args.ts";
import { EXIT, writeErr, writeOut } from "../output.ts";

export const usage = `bffi build [--config <p>] [--skip-build] [--debug]`;

export async function build(argv: string[]): Promise<number> {
  const args = parseArgs(argv);
  const config = flagString(args, "config");
  const skipBuild = flagBool(args, "skip-build");
  const debug = flagBool(args, "debug");

  try {
    const root = config === undefined ? process.cwd() : rootFromConfigPath(config);
    const cfg = await loadConfigFile(root);
    const api = await bffi({ config, skipBuild, debug });
    if (api === undefined) {
      writeErr("build: the pipeline returned no API object");
      return EXIT.fail;
    }
    writeOut(`ok: artifact ${localArtifactPath(cfg, root)}`);
    return EXIT.ok;
  } catch (error) {
    writeErr(`build failed: ${String(error)}`);
    return EXIT.fail;
  }
}
