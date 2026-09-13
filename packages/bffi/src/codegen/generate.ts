/**
 * The deterministic codegen renderer: a validated loader-schema JSON
 * in, one typed TS module out. The output embeds the schema as a
 * `const ... as const satisfies ModuleJson` literal - the `ApiOf<>`
 * mapping in `@z2net/bffi` derives the exact TypeScript signatures
 * from that literal, so the renderer emits NO hand-written types.
 *
 * Format contract (mirrors `bffi-dts::render`): a fixed header, LF
 * line endings, one trailing newline, no timestamps, paths or
 * versions. The embedded literal is CANONICALIZED (fixed field order,
 * declaration order preserved), so two inputs that differ only in key
 * order render identically.
 */
import { validateModule } from "./schema.ts";
import { BFFI_DIR } from "../pipeline/paths.ts";
import type { BffiConfig } from "../pipeline/config.ts";

/** The default module specifier the generated file imports the
 * runtime from: the package ITSELF (generation happens inside
 * `@z2net/bffi`, so the self-name is always resolvable - no relative
 * paths in generated output, ever). */
export const DEFAULT_RUNTIME = "@z2net/bffi";

/** One canonicalized schema object: fixed key order, no extras. */
type Json = string | number | boolean | null | Json[] | { [key: string]: Json };

function canonStringArray(value: unknown): Json {
  return (value as string[]).slice();
}

function canonParams(params: unknown): Json {
  return (params as { name: string; ts: string; abi: string }[]).map((param) => ({
    name: param.name,
    ts: param.ts,
    abi: param.abi,
  }));
}

function canonRet(ret: unknown): Json {
  const record = ret as { ts: string; abi: string };
  return { ts: record.ts, abi: record.abi };
}

function canonFunction(fn: unknown): Json {
  const record = fn as {
    name: string;
    export: string;
    docs: string[];
    params: unknown;
    ret: unknown;
    out?: string;
  };
  const out: { [key: string]: Json } = {
    name: record.name,
    export: record.export,
    docs: canonStringArray(record.docs),
    params: canonParams(record.params),
    ret: canonRet(record.ret),
  };
  if (record.out !== undefined) {
    out.out = record.out;
  }
  return out;
}

function canonField(field: unknown): Json {
  const record = field as {
    name: string;
    export: string;
    docs: string[];
    ts: string;
    out: string;
  };
  return {
    name: record.name,
    export: record.export,
    docs: canonStringArray(record.docs),
    ts: record.ts,
    out: record.out,
  };
}

function canonClass(cls: unknown): Json {
  const record = cls as {
    name: string;
    release: string;
    docs: string[];
    constructor: unknown;
    fields: unknown[];
    methods: unknown[];
  };
  return {
    name: record.name,
    release: record.release,
    docs: canonStringArray(record.docs),
    constructor: canonFunction(record.constructor),
    fields: (record.fields as unknown[]).map(canonField),
    methods: (record.methods as unknown[]).map(canonFunction),
  };
}

function canonRecordField(field: unknown): Json {
  const record = field as { name: string; docs: string[]; ts: string };
  return {
    name: record.name,
    docs: canonStringArray(record.docs),
    ts: record.ts,
  };
}

function canonRecord(entry: unknown): Json {
  const record = entry as { name: string; docs: string[]; fields: unknown[] };
  return {
    name: record.name,
    docs: canonStringArray(record.docs),
    fields: (record.fields as unknown[]).map(canonRecordField),
  };
}

function canonVariant(variant: unknown): Json {
  const record = variant as { name: string; docs: string[] };
  return {
    name: record.name,
    docs: canonStringArray(record.docs),
  };
}

function canonEnum(entry: unknown): Json {
  const record = entry as { name: string; docs: string[]; variants: unknown[] };
  return {
    name: record.name,
    docs: canonStringArray(record.docs),
    variants: (record.variants as unknown[]).map(canonVariant),
  };
}

function canonErrorField(field: unknown): Json {
  const record = field as { name: string; docs: string[]; ts: string };
  return {
    name: record.name,
    docs: canonStringArray(record.docs),
    ts: record.ts,
  };
}

function canonErrorVariant(variant: unknown): Json {
  const record = variant as {
    name: string;
    docs: string[];
    code: string;
    fields: unknown[];
  };
  return {
    name: record.name,
    docs: canonStringArray(record.docs),
    code: record.code,
    fields: (record.fields as unknown[]).map(canonErrorField),
  };
}

function canonError(entry: unknown): Json {
  const record = entry as { name: string; docs: string[]; variants: unknown[] };
  return {
    name: record.name,
    docs: canonStringArray(record.docs),
    variants: (record.variants as unknown[]).map(canonErrorVariant),
  };
}

/** Rebuilds the module in the canonical field order. */
function canonModule(raw: unknown): Json {
  const module = validateModule(raw);
  return {
    bffi: module.bffi,
    module: module.module,
    functions: module.functions.map(canonFunction),
    classes: module.classes.map(canonClass),
    records: module.records.map(canonRecord),
    enums: module.enums.map(canonEnum),
    errors: module.errors.map(canonError),
  };
}

/** Pretty-prints with two-space indentation, no trailing whitespace. */
function pretty(value: Json, indent: number): string {
  const pad = "  ".repeat(indent);
  const padInner = "  ".repeat(indent + 1);
  if (Array.isArray(value)) {
    if (value.length === 0) {
      return "[]";
    }
    const items = value.map((item) => `${padInner}${pretty(item, indent + 1)}`);
    return `[\n${items.join(",\n")}\n${pad}]`;
  }
  if (typeof value === "object" && value !== null) {
    const entries = Object.entries(value);
    if (entries.length === 0) {
      return "{}";
    }
    const items = entries.map(
      ([key, item]) => `${padInner}${JSON.stringify(key)}: ${pretty(item, indent + 1)}`,
    );
    return `{\n${items.join(",\n")}\n${pad}}`;
  }
  return JSON.stringify(value);
}

/**
 * Renders the validated raw schema into the generated TS module.
 *
 * Throws [`SchemaValidationError`] (from the validator) on malformed
 * input; the render itself never fails and is deterministic.
 */
export function renderModule(raw: unknown, runtime: string = DEFAULT_RUNTIME): string {
  const module = canonModule(raw);
  const moduleName = (module as { module: string }).module;
  const header =
    `// @generated by bffi codegen - do not edit.\n` +
    `// Source module: ${moduleName}\n\n` +
    `import { createApi, type ApiOf, type ModuleJson } from ${JSON.stringify(runtime)};\n\n`;
  const body =
    `const moduleJson = ${pretty(module, 0)} as const satisfies ModuleJson;\n\n` +
    `/** Opens the native library at \`libraryPath\` and returns the typed API. */\n` +
    `export function createApiFromJson(libraryPath: string): ApiOf<typeof moduleJson> {\n` +
    `  return createApi(moduleJson, libraryPath);\n` +
    `}\n\n` +
    `export type Api = ReturnType<typeof createApiFromJson>;\n\n` +
    `export { moduleJson };\n`;
  return header + body;
}

/**
 * The generation step of the pipeline: reads the loader JSON working
 * file (`.bffi/<files[0]>`, default `bffi.api.json`) and writes the
 * generated module (`.bffi/<generate.outFile>`, default
 * `api.gen.ts`) under `projectRoot`.
 *
 * The write is skipped when the content is byte-identical, so a
 * re-run does not invalidate module caches.
 *
 * Returns the absolute path of the generated file.
 */
export async function generateApiGen(config: BffiConfig, projectRoot: string): Promise<string> {
  const dir = `${projectRoot.replace(/\/+$/, "")}/${BFFI_DIR}`;
  const inputName = config.files[0] ?? "bffi.api.json";
  const outName = config.generate?.outFile ?? "api.gen.ts";

  const raw = JSON.parse(await Bun.file(`${dir}/${inputName}`).text());
  const rendered = renderModule(raw, DEFAULT_RUNTIME);

  const outPath = `${dir}/${outName}`;
  const existing = await Bun.file(outPath).exists()
    ? await Bun.file(outPath).text()
    : undefined;
  if (existing !== rendered) {
    await Bun.write(outPath, rendered);
  }
  return outPath;
}
