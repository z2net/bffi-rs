/**
 * The canonical model of the loader schema: the validated JSON is
 * rebuilt in a fixed field order (declaration order preserved, no
 * extras) so two inputs that differ only in key order render
 * identically, plus the typed views (`CanonModule`/`CanonFn`/
 * `CanonClass`) the renderer reads and the stable pretty-printer the
 * generated literal is embedded with.
 *
 * Split out of `generate.ts` (the renderer): canonicalization is a
 * pure data pass over the schema, the renderer is a pure data pass
 * over the canonical model.
 */

import { validateModule } from "#bffi/codegen/schema.ts";

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

/** Rebuilds the module in the canonical field order: the header is
 * `bffi`, `module`, `abiVersion` (default 1 when absent in the
 * input), `exportsHash` (only when present), then the tables. */
export function canonModule(raw: unknown): Json {
  const module = validateModule(raw);
  const meta = module as unknown as { abiVersion?: unknown; exportsHash?: unknown };
  const out: { [key: string]: Json } = {
    bffi: module.bffi,
    module: module.module,
    abiVersion: meta.abiVersion === undefined ? 1 : (meta.abiVersion as Json),
  };
  if (meta.exportsHash !== undefined) {
    out.exportsHash = meta.exportsHash as Json;
  }
  out.functions = module.functions.map(canonFunction);
  out.classes = module.classes.map(canonClass);
  out.records = module.records.map(canonRecord);
  out.enums = module.enums.map(canonEnum);
  out.errors = module.errors.map(canonError);
  return out;
}

/** Pretty-prints with two-space indentation, no trailing whitespace. */
export function pretty(value: Json, indent: number): string {
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


// ---------------------------------------------------------------------------
// Specialized-factory emission
// ---------------------------------------------------------------------------

/** The typed view of one canonical function descriptor. */
export interface CanonFn {
  name: string;
  export: string;
  docs: string[];
  params: { name: string; ts: string; abi: string }[];
  ret: { ts: string; abi: string };
  out?: string;
}

/** The typed view of one canonical class descriptor. */
export interface CanonClass {
  name: string;
  release: string;
  docs: string[];
  constructor: CanonFn;
  fields: { name: string; export: string; docs: string[]; ts: string; out: string }[];
  methods: CanonFn[];
}

/** The typed view of the canonical module. */
export interface CanonModule {
  bffi: number;
  module: string;
  functions: CanonFn[];
  classes: CanonClass[];
  records: { name: string; docs: string[]; fields: { name: string; docs: string[]; ts: string }[] }[];
  enums: { name: string; docs: string[]; variants: { name: string; docs: string[] }[] }[];
  errors: unknown[];
}
