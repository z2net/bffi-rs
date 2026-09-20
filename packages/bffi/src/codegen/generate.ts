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
import {
  canonModule,
  pretty,
  type CanonClass,
  type CanonFn,
  type CanonModule,
} from "./canonical.ts";
import { isCompositeTs, tablesOf } from "../loader/composite.ts";
import type { ModuleJson } from "../loader/loader.ts";

/** The default module specifier the generated file imports the
 * runtime from: the package ITSELF (generation happens inside
 * `@z2net/bffi`, so the self-name is always resolvable - no relative
 * paths in generated output, ever). */
export const DEFAULT_RUNTIME = "@z2net/bffi";

/** Runtime names the generated factory may import. Type-only names
 * are emitted with a `type ` prefix (verbatimModuleSyntax). */
type RuntimeName =
  | "ApiOf"
  | "ErrorCode"
  | "FfiLib"
  | "FfiSymbol"
  | "ModuleJson"
  | "TsOf"
  | "buildDeclarations"
  | "decodeAt"
  | "decodeUtf8"
  | "encodeValue"
  | "jsToWire"
  | "makeReadBuffer"
  | "makeRelease"
  | "makeTakeError"
  | "streamItemTs"
  | "tablesOf"
  | "wireToJs"
  | "wrapStream"
  | "wrapTask";

const TYPE_ONLY_IMPORTS: ReadonlySet<RuntimeName> = new Set<RuntimeName>([
  "ApiOf",
  "FfiLib",
  "FfiSymbol",
  "ModuleJson",
  "TsOf",
]);

/** Which runtime imports and factory plumbing the module needs. */
interface Needs {
  runtimeImports: Set<RuntimeName>;
  /** Any `ptr_len` parameter (borrowed pointers cross the call). */
  needPtr: boolean;
  /** Any buffer return (the `readBuffer` plumbing). */
  needReadBuffer: boolean;
}

/** Scans the canonical module for the features the factory uses. */
function collectNeeds(m: CanonModule, tables: ReturnType<typeof tablesOf>): Needs {
  const needs: Needs = {
    runtimeImports: new Set<RuntimeName>([
      "ApiOf",
      "ErrorCode",
      "FfiLib",
      "FfiSymbol",
      "ModuleJson",
      "TsOf",
      "buildDeclarations",
      "makeTakeError",
    ]),
    needPtr: false,
    needReadBuffer: false,
  };
  for (const fn of m.functions) {
    collectFnNeeds(fn, tables, needs);
  }
  for (const cls of m.classes) {
    needs.runtimeImports.add("makeRelease");
    collectFnNeeds(cls.constructor, tables, needs);
    for (const method of cls.methods) {
      collectFnNeeds(method, tables, needs);
    }
  }
  return needs;
}

function collectFnNeeds(
  fn: CanonFn,
  tables: ReturnType<typeof tablesOf>,
  needs: Needs,
): void {
  for (const param of fn.params) {
    if (param.abi === "ptr_len") {
      needs.needPtr = true;
      if (isCompositeTs(param.ts, tables)) {
        needs.runtimeImports.add("encodeValue");
        needs.runtimeImports.add("jsToWire");
        needs.runtimeImports.add("tablesOf");
      }
    }
  }
  switch (fn.ret.abi) {
    case "task":
      needs.runtimeImports.add("wrapTask");
      break;
    case "stream":
      needs.runtimeImports.add("wrapStream");
      needs.runtimeImports.add("streamItemTs");
      break;
    case "buffer": {
      needs.needReadBuffer = true;
      needs.runtimeImports.add("makeReadBuffer");
      if (fn.ret.ts === "string" || fn.ret.ts === "string | null") {
        needs.runtimeImports.add("decodeUtf8");
      }
      const bare = fn.ret.ts.endsWith(" | null")
        ? fn.ret.ts.slice(0, -" | null".length)
        : fn.ret.ts;
      if (isCompositeTs(bare, tables)) {
        needs.runtimeImports.add("decodeAt");
        needs.runtimeImports.add("tablesOf");
        needs.runtimeImports.add("wireToJs");
      }
      break;
    }
    default:
      break;
  }
}


/** The `bun:ffi` TypedArray constructor of one out-slot name. */
function outArrayType(out: string): string {
  switch (out) {
    case "i8":
      return "Int8Array";
    case "i16":
      return "Int16Array";
    case "i32":
      return "Int32Array";
    case "u8":
    case "bool":
      return "Uint8Array";
    case "u16":
      return "Uint16Array";
    case "u32":
      return "Uint32Array";
    case "f32":
      return "Float32Array";
    case "f64":
      return "Float64Array";
    case "i64":
      return "BigInt64Array";
    case "u64":
    case "handle":
      return "BigUint64Array";
    default:
      throw new Error(`unknown out slot: ${out}`);
  }
}

/** The expression reading one out slot back per its name (the
 * `readOut` semantics: handle/u64/i64 are bigints, bool a boolean,
 * everything else a number). */
function outRead(out: string, slotId: string): string {
  if (out === "bool") {
    return `(${slotId}[0] ?? 0) !== 0`;
  }
  if (out === "handle" || out === "u64" || out === "i64") {
    return `(${slotId}[0] ?? 0n)`;
  }
  return `(${slotId}[0] ?? 0)`;
}

/** A safe local identifier for a schema-named wrapper. */
function ident(prefix: string, name: string): string {
  const sanitized = name.replace(/[^A-Za-z0-9_$]/g, "_");
  const lead = /^[A-Za-z_$]/.test(sanitized) ? sanitized : `_${sanitized}`;
  return `${prefix}_${lead}`;
}

/** The `TsOf<...>` annotation for one schema `ts` name: the same
 * type-level mapping the consumer's `ApiOf` uses, so `tsc` verifies
 * every emitted signature against the embedded literal. */
function tsAnnotation(ts: string): string {
  return `TsOf<${JSON.stringify(ts)}, typeof moduleJson>`;
}

/** Emits one hoisted out-slot + symbol pair into factory scope (the
 * out slot only when the descriptor declares one). */
function emitHoist(
  lines: string[],
  fn: { export: string; out?: string },
  id: string,
): void {
  if (fn.out !== undefined) {
    lines.push(`  const ${ident("out", id)} = new ${outArrayType(fn.out)}(1);`);
  }
  lines.push(`  const ${ident("sym", id)} = sym(${JSON.stringify(fn.export)});`);
}

/** Emits the hoisted stream-item annotation of a stream-returning
 * descriptor (computed once, guarded once). */
function emitStreamHoist(lines: string[], fn: CanonFn, id: string): void {
  lines.push(`  const ${ident("stream_item", id)} = streamItemTs(${JSON.stringify(fn.ret.ts)});`);
  lines.push(`  if (${ident("stream_item", id)} === null) {`);
  lines.push(
    `    throw new Error(${JSON.stringify(
      `${fn.name}: stream return type must be AsyncIterableIterator<T>, got ${fn.ret.ts}`,
    )});`,
  );
  lines.push(`  }`);
}

/** Emits the argument-preparation lines and the native call
 * arguments of one parameter list. */
function emitArgs(
  lines: string[],
  fn: CanonFn,
  tables: ReturnType<typeof tablesOf>,
  needs: Needs,
  indent: string,
): string[] {
  const callArgs: string[] = [];
  for (const [index, param] of fn.params.entries()) {
    const value = `a${String(index)}`;
    const ctx = `${fn.name}(${param.name})`;
    switch (param.abi) {
      case "bool":
        lines.push(`${indent}if (typeof ${value} !== "boolean") {`);
        lines.push(`${indent}  throw new TypeError(${JSON.stringify(`${ctx}: expected boolean`)});`);
        lines.push(`${indent}}`);
        callArgs.push(`${value} ? 1 : 0`);
        break;
      case "ptr_len": {
        if (isCompositeTs(param.ts, tables)) {
          const wire = ident("wire", `${fn.name}_${param.name}`);
          lines.push(
            `${indent}const ${wire} = wireBytes(${JSON.stringify(param.ts)}, ${value}, ${JSON.stringify(ctx)});`,
          );
          callArgs.push(`${wire}.length > 0 ? ptr(${wire}) : null`, `${wire}.length`);
        } else {
          lines.push(`${indent}if (!(${value} instanceof Uint8Array)) {`);
          lines.push(
            `${indent}  throw new TypeError(${JSON.stringify(`${ctx}: expected ${param.ts}`)});`,
          );
          lines.push(`${indent}}`);
          callArgs.push(`${value}.length > 0 ? ptr(${value}) : null`, `${value}.length`);
        }
        break;
      }
      default:
        callArgs.push(value);
        break;
    }
  }
  return callArgs;
}

/** Emits the status-check + return-decode tail of one wrapper.
 * `callTarget` is the hoisted symbol; `callArgs` the prepared
 * arguments. */
function emitCallAndDecode(
  lines: string[],
  fn: CanonFn,
  id: string,
  tables: ReturnType<typeof tablesOf>,
  callArgs: string[],
  indent: string,
): void {
  const deep = `${indent}  `;
  if (fn.out !== undefined) {
    callArgs.push(ident("out", id));
  }
  lines.push(
    `${indent}const status = Number(${ident("sym", id)}(${callArgs.join(", ")}));`,
  );
  lines.push(`${indent}if (status !== ErrorCode.Ok) {`);
  lines.push(`${deep}const error = takeError(status);`);
  lines.push(
    `${deep}throw error ?? new Error("${fn.export} failed: " + String(status));`,
  );
  lines.push(`${indent}}`);
  const raw = fn.out !== undefined ? outRead(fn.out, ident("out", id)) : "undefined";
  switch (fn.ret.abi) {
    case "void":
      break;
    case "task":
      lines.push(
        `${indent}if (typeof (${raw}) !== "bigint") {`,
        `${deep}throw new TypeError(${JSON.stringify(`${fn.name}: expected a task handle`)});`,
        `${indent}}`,
        `${indent}return wrapTask(lib, ${raw}, ${JSON.stringify(fn.ret.ts)}, moduleJson) as unknown as ${tsAnnotation(fn.ret.ts)};`,
      );
      break;
    case "stream":
      lines.push(
        `${indent}if (typeof (${raw}) !== "bigint") {`,
        `${deep}throw new TypeError(${JSON.stringify(`${fn.name}: expected a stream handle`)});`,
        `${indent}}`,
        `${indent}return wrapStream(lib, ${raw}, ${ident("stream_item", id)}, moduleJson) as unknown as ${tsAnnotation(fn.ret.ts)};`,
      );
      break;
    case "buffer": {
      const { ts } = fn.ret;
      const nullable = ts.endsWith(" | null");
      lines.push(
        `${indent}if (typeof (${raw}) !== "bigint") {`,
        `${deep}throw new TypeError(${JSON.stringify(`${fn.name}: expected a buffer handle`)});`,
        `${indent}}`,
      );
      if (nullable) {
        lines.push(`${indent}if (${raw} === 0n) {`, `${deep}return null;`, `${indent}}`);
      }
      const bytesVar = ident("bytes", fn.name);
      lines.push(`${indent}const ${bytesVar} = readBuffer(${raw});`);
      const bare = nullable ? ts.slice(0, -" | null".length) : ts;
      if (isCompositeTs(bare, tables)) {
        lines.push(
          `${indent}return wireToJs(tables, ${JSON.stringify(bare)}, decodeAt(${bytesVar}, 0).value, ${JSON.stringify(
            `${fn.name}()`,
          )}) as ${tsAnnotation(ts)};`,
        );
      } else if (ts === "string") {
        lines.push(`${indent}return decodeUtf8(${bytesVar});`);
      } else if (ts === "string | null") {
        lines.push(`${indent}return ${bytesVar}.length === 0 ? null : decodeUtf8(${bytesVar});`);
      } else if (ts === "Uint8Array | null") {
        lines.push(`${indent}return ${bytesVar}.length === 0 ? null : ${bytesVar};`);
      } else {
        lines.push(`${indent}return ${bytesVar};`);
      }
      break;
    }
    // `handle` and every primitive width pass through as-is.
    default:
      lines.push(`${indent}return ${raw};`);
      break;
  }
}

/** The parameter list of one wrapper signature. */
function emitSignature(fn: CanonFn): string {
  const params = fn.params.map((param, index) => `a${String(index)}: ${tsAnnotation(param.ts)}`);
  const ret = fn.ret.abi === "void" ? "void" : tsAnnotation(fn.ret.ts);
  return `(${params.join(", ")}): ${ret}`;
}

/** Emits one top-level function wrapper (hoisted symbol and out
 * slot, fixed arity, inline encode/decode). */
function emitFunction(
  lines: string[],
  fn: CanonFn,
  tables: ReturnType<typeof tablesOf>,
  needs: Needs,
): void {
  emitHoist(lines, fn, fn.name);
  if (fn.ret.abi === "stream") {
    emitStreamHoist(lines, fn, fn.name);
  }
  const name = ident("fn", fn.name);
  lines.push(`  const ${name} = function ${emitSignature(fn)} {`);
  lines.push(
    `    if (arguments.length !== ${String(fn.params.length)}) {`,
    `      throw new Error(${JSON.stringify(
      `${fn.name}: expected ${String(fn.params.length)} argument(s), got `,
    )} + arguments.length);`,
    `    }`,
  );
  const callArgs = emitArgs(lines, fn, tables, needs, "    ");
  emitCallAndDecode(lines, fn, fn.name, tables, callArgs, "    ");
  lines.push(`  };`);
}

/** Emits one class wrapper: a real class body with the constructor,
 * field getters and methods specialized in place. */
function emitClass(
  lines: string[],
  cls: CanonClass,
  tables: ReturnType<typeof tablesOf>,
  needs: Needs,
): void {
  const clsId = ident("cls", cls.name);
  const relId = ident("release", cls.name);
  const finId = ident("finalizers", cls.name);
  emitHoist(lines, cls.constructor, `${cls.name}_${cls.constructor.name}`);
  for (const field of cls.fields) {
    emitHoist(lines, field, `${cls.name}_${field.name}`);
  }
  for (const method of cls.methods) {
    emitHoist(lines, method, `${cls.name}_${method.name}`);
    if (method.ret.abi === "stream") {
      emitStreamHoist(lines, method, `${cls.name}_${method.name}`);
    }
  }
  lines.push(`  const ${relId} = makeRelease(lib, ${JSON.stringify(cls.release)}, takeError);`);
  lines.push(`  const ${finId} = new FinalizationRegistry((handle: bigint): void => {`);
  lines.push(`    ${relId}(handle);`);
  lines.push(`  });`);
  lines.push(`  const ${clsId} = class {`);
  lines.push(`    handle: bigint;`);
  lines.push(`    [Symbol.dispose](): void {`);
  lines.push(`      this.release();`);
  lines.push(`    }`);
  // The constructor: parameters without a return annotation (TS
  // forbids one), the handle slot read into `this.handle` - the
  // ctor descriptor's `ret`/`out` pair never leaks as a `return`.
  lines.push(`    constructor(${cls.constructor.params.map((param, index) => `a${String(index)}: ${tsAnnotation(param.ts)}`).join(", ")}) {`);
  lines.push(
    `      if (arguments.length !== ${String(cls.constructor.params.length)}) {`,
    `        throw new Error(${JSON.stringify(
      `${cls.name} constructor: expected ${String(cls.constructor.params.length)} argument(s), got `,
    )} + arguments.length);`,
    `      }`,
  );
  const ctorArgs = emitArgs(lines, cls.constructor, tables, needs, "      ");
  const ctorId = `${cls.name}_${cls.constructor.name}`;
  if (cls.constructor.out !== undefined) {
    ctorArgs.push(ident("out", ctorId));
  }
  lines.push(
    `      const status = Number(${ident("sym", ctorId)}(${ctorArgs.join(", ")}));`,
  );
  lines.push(`      if (status !== ErrorCode.Ok) {`);
  lines.push(`        const error = takeError(status);`);
  lines.push(
    `        throw error ?? new Error("${cls.constructor.export} failed: " + String(status));`,
  );
  lines.push(`      }`);
  lines.push(`      this.handle = ${outRead(cls.constructor.out ?? "handle", ident("out", ctorId))};`);
  lines.push(`      ${finId}.register(this, this.handle);`);
  lines.push(`    }`);
  // release()
  lines.push(`    release(): void {`);
  lines.push(`      ${finId}.unregister(this);`);
  lines.push(`      ${relId}(this.handle);`);
  lines.push(`    }`);
  // Field getters (pure out-slot reads).
  for (const field of cls.fields) {
    const fieldId = `${cls.name}_${field.name}`;
    lines.push(`    get ${field.name}(): ${tsAnnotation(field.ts)} {`);
    lines.push(
      `      const status = Number(${ident("sym", fieldId)}(this.handle, ${ident("out", fieldId)}));`,
    );
    lines.push(`      if (status !== ErrorCode.Ok) {`);
    lines.push(`        const error = takeError(status);`);
    lines.push(
      `        throw error ?? new Error("${field.export} failed: " + String(status));`,
    );
    lines.push(`      }`);
    lines.push(`      return ${outRead(field.out, ident("out", fieldId))};`);
    lines.push(`    }`);
  }
  // Methods (the instance handle is prepended, NOT in the signature).
  for (const method of cls.methods) {
    const methodId = `${cls.name}_${method.name}`;
    lines.push(`    ${method.name}${emitSignature(method)} {`);
    lines.push(
      `      if (arguments.length !== ${String(method.params.length)}) {`,
      `        throw new Error(${JSON.stringify(
        `${cls.name}.${method.name}: expected ${String(method.params.length)} argument(s), got `,
      )} + arguments.length);`,
      `      }`,
    );
    const callArgs = emitArgs(lines, method, tables, needs, "      ");
    callArgs.unshift("this.handle");
    emitCallAndDecode(lines, method, methodId, tables, callArgs, "      ");
    lines.push(`    }`);
  }
  lines.push(`  };`);
}

/** Renders the whole factory body (everything inside the braces of
 * `createApiFromJson`). */
function renderFactory(m: CanonModule, needs: Needs, tables: ReturnType<typeof tablesOf>): string {
  const lines: string[] = [];
  lines.push(
    `  const lib: FfiLib = dlopen(libraryPath, buildDeclarations(moduleJson)).symbols as FfiLib;`,
  );
  lines.push(`  const takeError = makeTakeError(lib);`);
  if (needs.needReadBuffer) {
    lines.push(`  const readBuffer = makeReadBuffer(lib);`);
  }
  lines.push(`  const sym = (name: string): FfiSymbol => {`);
  lines.push(`    const found = lib[name];`);
  lines.push(`    if (typeof found !== "function") {`);
  lines.push(`      throw new Error("missing export: " + name);`);
  lines.push(`    }`);
  lines.push(`    return found;`);
  lines.push(`  };`);
  if (needs.runtimeImports.has("tablesOf")) {
    lines.push(`  const tables = tablesOf(moduleJson);`);
  }
  if (needs.runtimeImports.has("jsToWire")) {
    lines.push(`  const wireBytes = (ts: string, value: unknown, ctx: string): Uint8Array => {`);
    lines.push(`    const record = jsToWire(tables, ts, value, ctx);`);
    lines.push(`    const bytes: number[] = [];`);
    lines.push(`    encodeValue(bytes, record);`);
    lines.push(`    return new Uint8Array(bytes);`);
    lines.push(`  };`);
  }
  for (const fn of m.functions) {
    emitFunction(lines, fn, tables, needs);
  }
  for (const cls of m.classes) {
    emitClass(lines, cls, tables, needs);
  }
  lines.push(`  return {`);
  for (const fn of m.functions) {
    lines.push(`    ${JSON.stringify(fn.name)}: ${ident("fn", fn.name)},`);
  }
  for (const cls of m.classes) {
    lines.push(`    ${JSON.stringify(cls.name)}: ${ident("cls", cls.name)},`);
  }
  lines.push(`  };`);
  return `${lines.join("\n")}\n`;
}

/**
 * Renders the validated raw schema into the generated TS module.
 *
 * The generated factory is SPECIALIZED: one wrapper per function,
 * method, field and constructor with the symbol and the out-slot
 * TypedArray hoisted to factory scope, the argument encoding and the
 * return decoding inlined per ABI kind, and exact positional
 * parameters - no rest arrays, no per-call out allocation, no
 * per-call symbol lookup. Parameter and return types are annotated
 * through `TsOf<...>` against the embedded literal, so `tsc`
 * verifies every emitted signature with the same `ApiOf` mapping the
 * consumer sees.
 *
 * Throws [`SchemaValidationError`] (from the validator) on malformed
 * input; the render itself never fails and is deterministic.
 */
export function renderModule(raw: unknown, runtime: string = DEFAULT_RUNTIME): string {
  const module = canonModule(raw);
  const m = module as unknown as CanonModule;
  const tables = tablesOf(module as unknown as ModuleJson);
  const needs = collectNeeds(m, tables);
  const imports = [...needs.runtimeImports]
    .toSorted()
    .map((name) => `  ${TYPE_ONLY_IMPORTS.has(name) ? "type " : ""}${name},`)
    .join("\n");
  const header =
    `// @generated by bffi codegen - do not edit.\n` +
    `// Source module: ${m.module}\n` +
    `// Wrapper style: specialized (hoisted symbols and out slots,\n` +
    `// inline argument encoding and return decoding per ABI kind).\n\n` +
    `import { dlopen${needs.needPtr ? ", ptr" : ""} } from "bun:ffi";\n` +
    `import {\n${imports}\n} from ${JSON.stringify(runtime)};\n\n`;
  const body =
    `const moduleJson = ${pretty(module, 0)} as const satisfies ModuleJson;\n\n` +
    `/** The explicit low-level loader: opens the native library at\n` +
    ` * \`libraryPath\` and returns the typed API. Passing a raw binary\n` +
    ` * path is a trust decision - the pipeline resolves platform\n` +
    ` * packages by default. */\n` +
    `export function createApiFromJson(libraryPath: string): ApiOf<typeof moduleJson> {\n` +
    renderFactory(m, needs, tables) +
    `}\n\n` +
    `export type Api = ReturnType<typeof createApiFromJson>;\n\n` +
    `export { moduleJson };\n`;
  return header + body;
}

