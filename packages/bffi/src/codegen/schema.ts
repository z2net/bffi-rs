/**
 * The loader-schema validator used by the codegen CLI: a full
 * structural pass over the parsed JSON with precise `path: message`
 * diagnostics. Validation is value-level (unknown `abi`/`ts` names
 * are rejected) so the renderer can trust the shape afterwards.
 */
import {
  ABI_NAMES,
  OUT_NAMES,
  RET_ABI_NAMES,
  TS_NAMES,
} from "#bffi/loader/loader.ts";

/** One validation failure: the JSON path plus the reason. */
export interface SchemaIssue {
  path: string;
  message: string;
}

/** Raised by `validateModule`: `issues` is never empty. */
export class SchemaValidationError extends Error {
  readonly issues: SchemaIssue[];

  constructor(issues: SchemaIssue[]) {
    super(
      issues
        .map((issue) => `${issue.path}: ${issue.message}`)
        .join("\n"),
    );
    this.name = "SchemaValidationError";
    this.issues = issues;
  }
}

const STRING = "a string";
const STRING_ARRAY = "an array of strings";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function checkString(issues: SchemaIssue[], path: string, value: unknown): void {
  if (typeof value !== "string") {
    issues.push({ path, message: `expected ${STRING}` });
  }
}

function checkStringArray(issues: SchemaIssue[], path: string, value: unknown): void {
  if (!Array.isArray(value) || value.some((item) => typeof item !== "string")) {
    issues.push({ path, message: `expected ${STRING_ARRAY}` });
  }
}

function checkEnum(
  issues: SchemaIssue[],
  path: string,
  value: unknown,
  allowed: ReadonlySet<string>,
): void {
  if (typeof value !== "string" || !allowed.has(value)) {
    issues.push({
      path,
      message: `unknown name ${JSON.stringify(value)} (expected one of ${[...allowed].join(", ")})`,
    });
  }
}

function checkParams(
  issues: SchemaIssue[],
  path: string,
  value: unknown,
  named: (ts: string) => boolean,
): void {
  if (!Array.isArray(value)) {
    issues.push({ path, message: "expected an array of parameter entries" });
    return;
  }
  for (const [index, param] of value.entries()) {
    const at = `${path}[${index}]`;
    if (!isRecord(param)) {
      issues.push({ path: at, message: "expected an object" });
      continue;
    }
    checkString(issues, `${at}.name`, param.name);
    checkTs(issues, `${at}.ts`, param.ts, named);
    checkEnum(issues, `${at}.abi`, param.abi, ABI_NAMES);
  }
}

function checkRet(
  issues: SchemaIssue[],
  path: string,
  value: unknown,
  named: (ts: string) => boolean,
): void {
  if (!isRecord(value)) {
    issues.push({ path, message: "expected an object" });
    return;
  }
  checkTs(issues, `${path}.ts`, value.ts, named);
  checkEnum(issues, `${path}.abi`, value.abi, RET_ABI_NAMES);
}

/** A `ts` name is a known literal, or a named composite (module
 * table entry, optionally the `[]` form). */
function checkTs(
  issues: SchemaIssue[],
  path: string,
  value: unknown,
  named: (ts: string) => boolean,
): void {
  if (typeof value === "string" && named(value)) {
    return;
  }
  checkEnum(issues, path, value, TS_NAMES);
}

const OUT_NAMES_SET: ReadonlySet<string> = OUT_NAMES;

function checkFunctionLike(
  issues: SchemaIssue[],
  path: string,
  value: unknown,
  named: (ts: string) => boolean,
): void {
  if (!isRecord(value)) {
    issues.push({ path, message: "expected an object" });
    return;
  }
  checkString(issues, `${path}.name`, value.name);
  checkString(issues, `${path}.export`, value.export);
  checkStringArray(issues, `${path}.docs`, value.docs);
  checkParams(issues, `${path}.params`, value.params, named);
  checkRet(issues, `${path}.ret`, value.ret, named);
  if (value.out !== undefined) {
    checkEnum(issues, `${path}.out`, value.out, OUT_NAMES_SET);
  }
}

function checkClass(
  issues: SchemaIssue[],
  path: string,
  value: unknown,
  named: (ts: string) => boolean,
): void {
  if (!isRecord(value)) {
    issues.push({ path, message: "expected an object" });
    return;
  }
  checkString(issues, `${path}.name`, value.name);
  checkString(issues, `${path}.release`, value.release);
  checkStringArray(issues, `${path}.docs`, value.docs);
  checkFunctionLike(issues, `${path}.constructor`, value.constructor, named);
  if (!Array.isArray(value.fields)) {
    issues.push({ path: `${path}.fields`, message: "expected an array of field entries" });
  } else {
    for (const [index, field] of value.fields.entries()) {
      const at = `${path}.fields[${index}]`;
      if (!isRecord(field)) {
        issues.push({ path: at, message: "expected an object" });
        continue;
      }
      checkString(issues, `${at}.name`, field.name);
      checkString(issues, `${at}.export`, field.export);
      checkStringArray(issues, `${at}.docs`, field.docs);
      checkTs(issues, `${at}.ts`, field.ts, named);
      checkEnum(issues, `${at}.out`, field.out, OUT_NAMES_SET);
    }
  }
  if (!Array.isArray(value.methods)) {
    issues.push({ path: `${path}.methods`, message: "expected an array of method entries" });
  } else {
    for (const [index, method] of value.methods.entries()) {
      checkFunctionLike(issues, `${path}.methods[${index}]`, method, named);
    }
  }
}

function checkRecords(issues: SchemaIssue[], path: string, value: unknown, named: (ts: string) => boolean): void {
  if (!Array.isArray(value)) {
    issues.push({ path, message: "expected an array of record entries" });
    return;
  }
  for (const [index, entry] of value.entries()) {
    const at = `${path}[${index}]`;
    if (!isRecord(entry)) {
      issues.push({ path: at, message: "expected an object" });
      continue;
    }
    checkString(issues, `${at}.name`, entry.name);
    checkStringArray(issues, `${at}.docs`, entry.docs);
    if (!Array.isArray(entry.fields)) {
      issues.push({ path: `${at}.fields`, message: "expected an array of field entries" });
    } else {
      for (const [fieldIndex, field] of entry.fields.entries()) {
        const fieldAt = `${at}.fields[${fieldIndex}]`;
        if (!isRecord(field)) {
          issues.push({ path: fieldAt, message: "expected an object" });
          continue;
        }
        checkString(issues, `${fieldAt}.name`, field.name);
        checkStringArray(issues, `${fieldAt}.docs`, field.docs);
        checkTs(issues, `${fieldAt}.ts`, field.ts, named);
      }
    }
  }
}

function checkEnums(issues: SchemaIssue[], path: string, value: unknown): void {
  if (!Array.isArray(value)) {
    issues.push({ path, message: "expected an array of enum entries" });
    return;
  }
  for (const [index, entry] of value.entries()) {
    const at = `${path}[${index}]`;
    if (!isRecord(entry)) {
      issues.push({ path: at, message: "expected an object" });
      continue;
    }
    checkString(issues, `${at}.name`, entry.name);
    checkStringArray(issues, `${at}.docs`, entry.docs);
    if (!Array.isArray(entry.variants)) {
      issues.push({ path: `${at}.variants`, message: "expected an array of variant entries" });
    } else {
      for (const [variantIndex, variant] of entry.variants.entries()) {
        const variantAt = `${at}.variants[${variantIndex}]`;
        if (!isRecord(variant)) {
          issues.push({ path: variantAt, message: "expected an object" });
          continue;
        }
        checkString(issues, `${variantAt}.name`, variant.name);
        checkStringArray(issues, `${variantAt}.docs`, variant.docs);
      }
    }
  }
}

/** The hex user-code shape carried by every error variant. */
const HEX_CODE = /^0x[0-9A-F]{4}$/;

function checkErrors(issues: SchemaIssue[], path: string, value: unknown): void {
  if (!Array.isArray(value)) {
    issues.push({ path, message: "expected an array of error entries" });
    return;
  }
  for (const [index, entry] of value.entries()) {
    const at = `${path}[${index}]`;
    if (!isRecord(entry)) {
      issues.push({ path: at, message: "expected an object" });
      continue;
    }
    checkString(issues, `${at}.name`, entry.name);
    checkStringArray(issues, `${at}.docs`, entry.docs);
    if (!Array.isArray(entry.variants)) {
      issues.push({ path: `${at}.variants`, message: "expected an array of variant entries" });
    } else {
      for (const [variantIndex, variant] of entry.variants.entries()) {
        const variantAt = `${at}.variants[${variantIndex}]`;
        if (!isRecord(variant)) {
          issues.push({ path: variantAt, message: "expected an object" });
          continue;
        }
        checkString(issues, `${variantAt}.name`, variant.name);
        checkStringArray(issues, `${variantAt}.docs`, variant.docs);
        if (
          typeof variant.code !== "string" ||
          !HEX_CODE.test(variant.code) ||
          Number.parseInt(variant.code, 16) < 0x1000
        ) {
          issues.push({
            path: `${variantAt}.code`,
            message: "expected a hex user code in the reserved range (0x1000..=0xFFFF)",
          });
        }
        if (variant.fields !== undefined && !Array.isArray(variant.fields)) {
          issues.push({ path: `${variantAt}.fields`, message: "expected an array of fields" });
        } else if (Array.isArray(variant.fields)) {
          for (const [fieldIndex, field] of variant.fields.entries()) {
            const fieldAt = `${variantAt}.fields[${fieldIndex}]`;
            if (!isRecord(field)) {
              issues.push({ path: fieldAt, message: "expected an object" });
              continue;
            }
            checkString(issues, `${fieldAt}.name`, field.name);
            checkStringArray(issues, `${fieldAt}.docs`, field.docs);
            checkString(issues, `${fieldAt}.ts`, field.ts);
          }
        }
      }
    }
  }
}

/**
 * Validates the raw parsed JSON against loader schema v1 and returns
 * it typed. Throws [`SchemaValidationError`] with every issue found.
 */
export function validateModule(raw: unknown): ModuleJsonLike {
  const issues: SchemaIssue[] = [];
  if (!isRecord(raw)) {
    throw new SchemaValidationError([
      { path: "$", message: "expected a JSON object" },
    ]);
  }
  if (raw.bffi !== 1) {
    issues.push({
      path: "$.bffi",
      message: `unsupported schema version ${JSON.stringify(raw.bffi)} (expected 1)`,
    });
  }
  checkString(issues, "$.module", raw.module);
  const records = Array.isArray(raw.records) ? raw.records : [];
  const enums = Array.isArray(raw.enums) ? raw.enums : [];
  const recordNames = new Set(
    records
      .filter(isRecord)
      .map((entry) => entry.name)
      .filter((name): name is string => typeof name === "string"),
  );
  const enumNames = new Set(
    enums
      .filter(isRecord)
      .map((entry) => entry.name)
      .filter((name): name is string => typeof name === "string"),
  );
  const named = (ts: string): boolean => {
    // A stream return wraps one element type (plain or the Result
    // form `<T | Error>`).
    if (ts.startsWith("AsyncIterableIterator<") && ts.endsWith(">")) {
      let inner = ts.slice("AsyncIterableIterator<".length, -1);
      if (inner.endsWith(" | Error")) {
        inner = inner.slice(0, -" | Error".length);
      }
      return named(inner);
    }
    // A promise return wraps one element type (`Promise<Sample>`).
    if (ts.startsWith("Promise<") && ts.endsWith(">")) {
      return named(ts.slice("Promise<".length, -1));
    }
    // A nullable return wraps one element type (`<T> | null`).
    if (ts.endsWith(" | null")) {
      return named(ts.slice(0, -" | null".length));
    }
    const name = ts.endsWith("[]") ? ts.slice(0, -2) : ts;
    if (TS_NAMES.has(name)) {
      return true;
    }
    return recordNames.has(name) || enumNames.has(name);
  };
  if (!Array.isArray(raw.functions)) {
    issues.push({ path: "$.functions", message: "expected an array" });
  } else {
    for (const [index, fn] of raw.functions.entries()) {
      checkFunctionLike(issues, `$.functions[${index}]`, fn, named);
    }
  }
  if (!Array.isArray(raw.classes)) {
    issues.push({ path: "$.classes", message: "expected an array" });
  } else {
    for (const [index, cls] of raw.classes.entries()) {
      checkClass(issues, `$.classes[${index}]`, cls, named);
    }
  }
  if (raw.records !== undefined) {
    checkRecords(issues, "$.records", raw.records, named);
  }
  if (raw.enums !== undefined) {
    checkEnums(issues, "$.enums", raw.enums);
  }
  if (raw.errors !== undefined) {
    checkErrors(issues, "$.errors", raw.errors);
  }
  if (issues.length > 0) {
    throw new SchemaValidationError(issues);
  }
  return {
    ...(raw as Record<string, unknown>),
    records: Array.isArray(raw.records) ? raw.records : [],
    enums: Array.isArray(raw.enums) ? raw.enums : [],
    errors: Array.isArray(raw.errors) ? raw.errors : [],
  } as unknown as ModuleJsonLike;
}

/** The minimal structural type the validator guarantees. */
export interface ModuleJsonLike {
  bffi: number;
  module: string;
  functions: unknown[];
  classes: unknown[];
  records: unknown[];
  enums: unknown[];
  errors: unknown[];
}
