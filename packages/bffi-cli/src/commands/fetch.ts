/** `bffi fetch --repo <owner/repo> --tag <v> --asset <file>
 * [--sha256 <hex>] [--allow-unsigned] [--out <dir>]`: downloads a
 * prebuilt binary from a GitHub Release. A digest is REQUIRED by
 * default: pass `--sha256 <hex>` or publish an `<asset>.sha256`
 * sidecar next to the release asset; `--allow-unsigned` is the
 * explicit escape hatch that skips integrity verification. */
import { joinOut } from "@z2net/bffi";
import { flagBool, flagString, parseArgs, type ParsedArgs } from "../args.ts";
import { EXIT, writeErr, writeOut } from "../output.ts";

export const usage =
  `bffi fetch --repo <owner/repo> --tag <v> --asset <file> [--sha256 <hex>] [--allow-unsigned] [--out <dir>]`;

/** How the digest check should run for one download. */
export interface DigestPolicy {
  mode: "required" | "allow-unsigned";
  /** The expected sha256 hex digest (lowercase); absent means "no
   * digest available" so the caller refuses to install. */
  digest?: string;
}

/** Pure digest decision: an explicit `--sha256` wins, then
 * `--allow-unsigned` disables verification (the sidecar is ignored),
 * then the sidecar digest applies, else "required" without a digest
 * (the caller must refuse to install). Digests are normalized to
 * lowercase hex. */
export function resolveDigestPolicy(
  args: ParsedArgs,
  sidecarDigest: string | undefined,
): DigestPolicy {
  const flagged = flagString(args, "sha256")?.trim().toLowerCase();
  if (flagged !== undefined && flagged !== "") {
    return { mode: "required", digest: flagged };
  }
  if (flagBool(args, "allow-unsigned")) {
    return { mode: "allow-unsigned" };
  }
  const sidecar = sidecarDigest?.trim().toLowerCase();
  if (sidecar !== undefined && sidecar !== "") {
    return { mode: "required", digest: sidecar };
  }
  return { mode: "required" };
}

/** The command handler. Named `fetchCmd` so it does not shadow the
 * global `fetch` used for downloads. */
export async function fetchCmd(argv: string[]): Promise<number> {
  const args = parseArgs(argv);
  const repo = flagString(args, "repo");
  const tag = flagString(args, "tag");
  const asset = flagString(args, "asset");
  const outDir = flagString(args, "out") ?? joinOut("target", "bffi");

  if (repo === undefined || tag === undefined || asset === undefined) {
    writeErr(usage);
    return EXIT.usage;
  }

  const baseUrl = `https://github.com/${repo}/releases/download/${tag}`;
  const response = await fetch(`${baseUrl}/${asset}`);
  if (!response.ok) {
    writeErr(`fetch: ${baseUrl}/${asset} -> HTTP ${String(response.status)}`);
    return EXIT.fail;
  }
  const bytes = new Uint8Array(await response.arrayBuffer());

  // The `<asset>.sha256` sidecar ("<hex>" or "<hex>  <filename>"),
  // fetched before the policy check so its absence can fail the
  // default "digest required" mode.
  let sidecarDigest: string | undefined;
  const hashResponse = await fetch(`${baseUrl}/${asset}.sha256`);
  if (hashResponse.ok) {
    sidecarDigest = (await hashResponse.text()).trim().split(/\s+/)[0];
  }

  const policy = resolveDigestPolicy(args, sidecarDigest);
  if (policy.mode === "allow-unsigned") {
    writeErr("fetch: downloading WITHOUT integrity verification (--allow-unsigned)");
  } else {
    if (policy.digest === undefined) {
      writeErr(`fetch: refusing to install ${asset} without a digest`);
      writeErr(`fetch: pass --sha256 <hex> or publish a ${asset}.sha256 sidecar in the release`);
      writeErr("fetch: (use --allow-unsigned to skip integrity verification explicitly)");
      return EXIT.fail;
    }
    const hasher = new Bun.CryptoHasher("sha256");
    hasher.update(bytes);
    const actual = hasher.digest("hex");
    if (actual !== policy.digest) {
      writeErr(`fetch: sha256 mismatch (expected ${policy.digest}, got ${actual})`);
      return EXIT.fail;
    }
  }

  const outPath = joinOut(process.cwd(), outDir, asset);
  await Bun.write(outPath, bytes);
  writeOut(`fetched ${asset} (${String(bytes.length)} bytes) -> ${outPath}`);
  return EXIT.ok;
}
