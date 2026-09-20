/**
 * The `@z2net/bffi-native` entry: resolves the prebuilt binary of
 * the RUNNING platform (installed as an optional dependency of the
 * main package, napi-rs style) and opens it through the
 * `@z2net/bffi` typed loader.
 *
 * An explicit `libraryPath` overrides the resolution (useful for
 * tests and locally built artifacts).
 */
import { resolvePlatformBinary } from "@z2net/bffi";

import { createApiFromJson, type Api } from "./api.gen.ts";

export type { Api };

/** Opens the prebuilt native library and returns the typed API.
 * Async because the resolution verifies the platform package's
 * integrity digest (`Bun.file` readers are async). */
export async function createNative(libraryPath?: string): Promise<Api> {
  return createApiFromJson(
    libraryPath ??
      (await resolvePlatformBinary("@z2net/bffi-native", { binary: "bffi_native" })),
  );
}
