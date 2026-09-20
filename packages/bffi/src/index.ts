/**
 * The public surface of `@z2net/bffi`: schema types, declaration
 * builder, wire codec, and the runtime primitives the generated
 * modules (and hand-rolled loaders) compose.
 *
 * The surface is re-exported 1:1 from the domain barrels
 * (runtime / loader / pipeline / codegen); see package.json
 * `exports` for the subpath entries.
 */
export {
  ErrorCode,
  makeTakeError,
  sym,
  type FfiLib,
  type FfiSymbol,
} from "./runtime/index.ts";
export { makeReadBuffer, makeFreeBuffer, decodeUtf8 } from "./runtime/index.ts";
export {
  TAG_UNIT,
  TAG_I32,
  TAG_I64,
  TAG_F64,
  TAG_BOOL,
  TAG_STR,
  TAG_BYTES,
  TAG_U64,
  MAX_WIRE_DEPTH,
  MAX_WIRE_PAYLOAD,
  setMaxWirePayload,
  decodeAt,
  decodeValue,
  encodeArgs,
  encodeValue,
  type WireValue,
} from "./runtime/index.ts";
export {
  SCHEMA_VERSION,
  BFFI_ABI_VERSION,
  assertSchema,
  buildDeclarations,
  type AbiName,
  type ClassJson,
  type FieldJson,
  type FunctionJson,
  type ModuleJson,
  type OutName,
  type ParamJson,
  type RetAbiName,
  type RetJson,
  type TsName,
} from "./loader/index.ts";
export { pumpUntil, wrapTask, wrapStream, streamItemTs, streamToWeb } from "./runtime/index.ts";
export {
  disposeLib,
  installMemoryPressureGC,
  makeLibDisposer,
} from "./runtime/index.ts";
export {
  createApi,
  createApiFromLib,
  makeRelease,
  isCompositeTs,
  jsToWire,
  tablesOf,
  wireToJs,
  type ApiOf,
  type ClassOf,
  type FnOf,
  type ParamsOf,
  type TsOf,
} from "./loader/index.ts";
export {
  bindJsCallback,
  invokeCallback,
  revokeCallback,
  setJsThread,
  unsetJsThread,
  type CallbackSig,
  type CbType,
  type CbValue,
} from "./runtime/index.ts";
import { assertBunVersion } from "./runtime/index.ts";
export {
  assertBunVersion,
  bunVersionProblem,
  bunVersionSatisfies,
  MIN_BUN_VERSION,
} from "./runtime/index.ts";
export {
  platformTriple,
  resolvePlatformBinary,
  tripleWithLibc,
  type ResolveOptions,
} from "./loader/index.ts";
export {
  joinOut,
  fileUrl,
  BFFI_DIR,
  CONFIG_FILE,
} from "./pipeline/index.ts";
export { DEFAULT_RUNTIME, renderModule } from "./codegen/index.ts";
export {
  validateModule,
  SchemaValidationError,
  type ModuleJsonLike,
  type SchemaIssue,
} from "./codegen/index.ts";
export {
  defineConfig,
  loadConfigFile,
  validateConfig,
  findProjectRoot,
  CONFIG_VERSION,
  ConfigValidationError,
  type BffiConfig,
  type CrateConfig,
  type ConfigIssue,
} from "./pipeline/index.ts";
export {
  applyDebug,
  debugLog,
  isDebug,
} from "./pipeline/index.ts";
export { buildCrate, type BuildOptions } from "./pipeline/index.ts";
export {
  bffi,
  bffiBuild,
  bffiGenerate,
  localArtifactPath,
  type BffiOptions,
  type GenerateOptions,
} from "./pipeline/index.ts";

// Fail fast on unsupported runtimes: every public consumer (the
// generated modules included) imports this entry.
assertBunVersion();
