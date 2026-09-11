/**
 * B1 composite conversions: JS values <-> wire values for the named
 * record/enum types and the array forms, resolved against the
 * module's `records`/`enums` tables.
 *
 * The conversions are strict: a `number` field rejects a string, a
 * `bigint` field rejects a `number` (the wire carries exact widths -
 * a JS number in an `i64` slot would silently encode as `i32`/`f64`
 * and break the Rust decode), an enum value must be one of the
 * declared variants. Errors name the offending path.
 */

import type { ModuleJson, RecordJson } from "./loader.ts";
import type { WireValue } from "../runtime/wire.ts";

/** The module tables with the B1 optional keys normalized to arrays. */
export interface CompositeTables {
  records: RecordJson[];
  enums: { name: string; variants: { name: string }[] }[];
}

/** Reads the composite tables off a module JSON (missing keys read as
 * empty - old-schema compatibility). */
export function tablesOf(json: ModuleJson): CompositeTables {
  return {
    records: json.records ?? [],
    enums: json.enums ?? [],
  };
}

/** Whether `ts` rides the wire channel (a named record/enum, their
 * array forms, or the flat arrays). `Uint8Array` is excluded: it is
 * the raw borrowed-buffer path. */
export function isCompositeTs(ts: string, tables: CompositeTables): boolean {
  if (
    ts === "number[]" || ts === "bigint[]" || ts === "boolean[]" || ts === "string[]"
  ) {
    return true;
  }
  const name = ts.endsWith("[]") ? ts.slice(0, -2) : ts;
  return (
    tables.records.some((record) => record.name === name)
    || tables.enums.some((enumeration) => enumeration.name === name)
  );
}

/** Converts one JS value into its wire form for `ts`. */
export function jsToWire(
  tables: CompositeTables,
  ts: string,
  value: unknown,
  path: string,
): WireValue {
  const fail = (expected: string): TypeError =>
    new TypeError(`${path}: expected ${expected}, got ${typeof value}`);
  if (ts === "number") {
    if (typeof value !== "number") {
      throw fail("number");
    }
    return value;
  }
  if (ts === "bigint") {
    if (typeof value !== "bigint") {
      throw fail("bigint");
    }
    return value;
  }
  if (ts === "boolean") {
    if (typeof value !== "boolean") {
      throw fail("boolean");
    }
    return value;
  }
  if (ts === "string") {
    if (typeof value !== "string") {
      throw fail("string");
    }
    return value;
  }
  if (ts === "Uint8Array") {
    if (!(value instanceof Uint8Array)) {
      throw fail("Uint8Array");
    }
    return value;
  }
  if (ts.endsWith("[]")) {
    if (!Array.isArray(value)) {
      throw fail("array");
    }
    const itemTs = ts.slice(0, -2);
    return value.map((item, index) => jsToWire(tables, itemTs, item, `${path}[${String(index)}]`));
  }
  const record = tables.records.find((entry) => entry.name === ts);
  if (record) {
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
      throw fail(`record ${ts}`);
    }
    const source = value as Record<string, unknown>;
    return {
      fields: record.fields.map((field) =>
        jsToWire(tables, field.ts, source[field.name], `${path}.${field.name}`)
      ),
    };
  }
  const enumeration = tables.enums.find((entry) => entry.name === ts);
  if (enumeration) {
    if (typeof value !== "string") {
      throw fail(`enum ${ts} (a variant name string)`);
    }
    if (!enumeration.variants.some((variant) => variant.name === value)) {
      throw new TypeError(
        `${path}: unknown ${ts} variant ${value} (expected one of ${enumeration.variants.map((v) => v.name).join(", ")})`,
      );
    }
    return value;
  }
  throw new Error(`${path}: unknown composite type ${ts}`);
}

/** Converts one decoded wire value back into its JS form for `ts`. */
export function wireToJs(
  tables: CompositeTables,
  ts: string,
  value: WireValue,
  path: string,
): unknown {
  if (ts === "number") {
    return value;
  }
  if (ts === "bigint") {
    return value;
  }
  if (ts === "boolean") {
    return value;
  }
  if (ts === "string") {
    return value;
  }
  if (ts === "Uint8Array") {
    return value;
  }
  if (ts.endsWith("[]")) {
    if (!Array.isArray(value)) {
      throw new Error(`${path}: expected a wire sequence`);
    }
    const itemTs = ts.slice(0, -2);
    return value.map((item, index) => wireToJs(tables, itemTs, item, `${path}[${String(index)}]`));
  }
  const record = tables.records.find((entry) => entry.name === ts);
  if (record) {
    if (typeof value !== "object" || value === null || Array.isArray(value) || !("fields" in value)) {
      throw new Error(`${path}: expected a wire record`);
    }
    const fields = (value as { fields: WireValue[] }).fields;
    if (fields.length !== record.fields.length) {
      throw new Error(
        `${path}: record ${ts} field count mismatch (wire ${String(fields.length)}, descriptor ${String(record.fields.length)})`,
      );
    }
    const out: Record<string, unknown> = {};
    record.fields.forEach((field, index) => {
      out[field.name] = wireToJs(tables, field.ts, fields[index], `${path}.${field.name}`);
    });
    return out;
  }
  const enumeration = tables.enums.find((entry) => entry.name === ts);
  if (enumeration) {
    return value;
  }
  throw new Error(`${path}: unknown composite type ${ts}`);
}
