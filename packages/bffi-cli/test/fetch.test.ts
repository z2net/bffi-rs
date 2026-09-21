/**
 * Tests for `bffi fetch`: a digest is REQUIRED by default (`--sha256`
 * flag or `<asset>.sha256` sidecar), `--allow-unsigned` is the
 * explicit escape hatch, and mismatches fail with both digests.
 *
 * The downloader uses the global `fetch`, so the HTTP layer is mocked
 * via `globalThis.fetch` and the handler is called directly (same
 * style as pack.test.ts).
 *
 * Run with `bun test packages/bffi-cli`.
 */
import { afterAll, afterEach, beforeEach, describe, expect, mock, test } from "bun:test";

import { parseArgs } from "#cli/args.ts";
import { fetchCmd, resolveDigestPolicy } from "#cli/commands/fetch.ts";

const ROOT = `${import.meta.dir}/tmp-fetch`;
const ASSET = "libdemo.so";
const BODY = new Uint8Array([1, 2, 3, 4, 5]);
const GOOD = new Bun.CryptoHasher("sha256").update(BODY).digest("hex");
const BAD = "0".repeat(64);

type FetchLike = (input: string | URL | Request) => Promise<Response>;
const realFetch = globalThis.fetch;
const fetchMock = mock<FetchLike>(() =>
  Promise.resolve(new Response(null, { status: 404 })),
);

/** Serves the asset body unless the URL targets the `.sha256` sidecar. */
function serve(asset: Response, sidecar: Response): void {
  fetchMock.mockImplementation((input) => {
    const url = String(input);
    return Promise.resolve(url.endsWith(".sha256") ? sidecar : asset);
  });
}

/** Runs `fetchCmd` with captured stdout/stderr. */
async function run(
  argv: string[],
): Promise<{ code: number; err: string; out: string }> {
  const err: string[] = [];
  const out: string[] = [];
  const origErr = process.stderr.write;
  const origOut = process.stdout.write;
  process.stderr.write = (chunk) => {
    err.push(String(chunk));
    return true;
  };
  process.stdout.write = (chunk) => {
    out.push(String(chunk));
    return true;
  };
  try {
    const code = await fetchCmd(argv);
    return { code, err: err.join(""), out: out.join("") };
  } finally {
    process.stderr.write = origErr;
    process.stdout.write = origOut;
  }
}

function baseArgs(outDir: string): string[] {
  return ["--repo", "acme/demo", "--tag", "v1", "--asset", ASSET, "--out", outDir];
}

describe("resolveDigestPolicy", () => {
  test("an explicit --sha256 wins over the sidecar and --allow-unsigned", () => {
    const policy = resolveDigestPolicy(
      parseArgs(["--sha256", ` ${BAD.toUpperCase()} `, "--allow-unsigned"]),
      GOOD,
    );
    expect(policy).toEqual({ mode: "required", digest: BAD });
  });

  test("--allow-unsigned disables verification and ignores the sidecar", () => {
    expect(resolveDigestPolicy(parseArgs(["--allow-unsigned"]), GOOD)).toEqual({
      mode: "allow-unsigned",
    });
  });

  test("the sidecar digest applies by default", () => {
    expect(resolveDigestPolicy(parseArgs([]), GOOD)).toEqual({
      mode: "required",
      digest: GOOD,
    });
  });

  test("no digest anywhere is still 'required' but undigestable", () => {
    expect(resolveDigestPolicy(parseArgs([]), undefined)).toEqual({
      mode: "required",
    });
  });

  test("sidecar digests are normalized (trim + lowercase)", () => {
    expect(resolveDigestPolicy(parseArgs([]), `  ${GOOD.toUpperCase()}\n`)).toEqual({
      mode: "required",
      digest: GOOD,
    });
  });
});

describe("bffi fetch", () => {
  beforeEach(() => {
    fetchMock.mockClear();
    globalThis.fetch = fetchMock as unknown as typeof globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  test("missing --repo/--tag/--asset is a usage error with no HTTP call", async () => {
    const { code } = await run(["--repo", "acme/demo"]);
    expect(code).toBe(1);
    expect(fetchMock.mock.calls.length).toBe(0);
  });

  test("refuses to install when neither --sha256 nor a sidecar exists", async () => {
    serve(
      new Response(BODY, { status: 200 }),
      new Response(null, { status: 404 }),
    );
    const { code, err } = await run(baseArgs(`${ROOT}/dl-refuse`));
    expect(code).toBe(2);
    expect(err).toContain("--sha256");
    expect(await Bun.file(`${ROOT}/dl-refuse/${ASSET}`).exists()).toBeFalse();
  });

  test("--allow-unsigned downloads with a loud warning", async () => {
    serve(
      new Response(BODY, { status: 200 }),
      new Response(null, { status: 404 }),
    );
    const { code, err } = await run([
      ...baseArgs(`${ROOT}/dl-unsigned`),
      "--allow-unsigned",
    ]);
    expect(code).toBe(0);
    expect(err).toContain(
      "fetch: downloading WITHOUT integrity verification (--allow-unsigned)",
    );
    expect(await Bun.file(`${ROOT}/dl-unsigned/${ASSET}`).exists()).toBeTrue();
  });

  test("a matching '<hex>  <filename>' sidecar installs", async () => {
    serve(
      new Response(BODY, { status: 200 }),
      new Response(`${GOOD}  ${ASSET}\n`, { status: 200 }),
    );
    const { code, err } = await run(baseArgs(`${ROOT}/dl-sidecar`));
    expect(code).toBe(0);
    expect(err).not.toContain("WITHOUT integrity verification");
    expect(await Bun.file(`${ROOT}/dl-sidecar/${ASSET}`).exists()).toBeTrue();
  });

  test("a sidecar mismatch fails with both digests", async () => {
    serve(
      new Response(BODY, { status: 200 }),
      new Response(BAD, { status: 200 }),
    );
    const { code, err } = await run(baseArgs(`${ROOT}/dl-sidecar-bad`));
    expect(code).toBe(2);
    expect(err).toContain(BAD);
    expect(err).toContain(GOOD);
    expect(await Bun.file(`${ROOT}/dl-sidecar-bad/${ASSET}`).exists()).toBeFalse();
  });

  test("--sha256 is honored when the sidecar is missing", async () => {
    serve(
      new Response(BODY, { status: 200 }),
      new Response(null, { status: 404 }),
    );
    const { code, err } = await run([
      ...baseArgs(`${ROOT}/dl-flag`),
      "--sha256",
      GOOD.toUpperCase(),
    ]);
    expect(code).toBe(0);
    expect(err).not.toContain("WITHOUT integrity verification");
    expect(await Bun.file(`${ROOT}/dl-flag/${ASSET}`).exists()).toBeTrue();
  });

  test("a --sha256 mismatch fails with both digests", async () => {
    serve(
      new Response(BODY, { status: 200 }),
      new Response(null, { status: 404 }),
    );
    const { code, err } = await run([
      ...baseArgs(`${ROOT}/dl-flag-bad`),
      "--sha256",
      BAD,
    ]);
    expect(code).toBe(2);
    expect(err).toContain(BAD);
    expect(err).toContain(GOOD);
  });

  test("--sha256 takes precedence over --allow-unsigned", async () => {
    serve(
      new Response(BODY, { status: 200 }),
      new Response(null, { status: 404 }),
    );
    const { code, err } = await run([
      ...baseArgs(`${ROOT}/dl-both`),
      "--sha256",
      BAD,
      "--allow-unsigned",
    ]);
    expect(code).toBe(2);
    expect(err).not.toContain("WITHOUT integrity verification");
    expect(err).toContain(BAD);
  });

  afterAll(async () => {
    const { $ } = await import("bun");
    await $`rm -rf ${ROOT}`;
  });
});
