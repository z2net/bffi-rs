/**
 * Unit tests for the loader package: wire codec round-trips against
 * the Rust-side vectors (bffi-async tests), declaration building,
 * argument encoding rules (null for empty buffers, bool coercion)
 * and the API factory over a mock symbol table.
 *
 * Run with `bun test packages/bffi`.
 */
import { describe, expect, test } from "bun:test";

import {
  TAG_BOOL,
  TAG_BYTES,
  TAG_F64,
  TAG_I32,
  TAG_I64,
  TAG_STR,
  TAG_UNIT,
  BFFI_ABI_VERSION,
  type ModuleJson,
  assertSchema,
  buildDeclarations,
  createApiFromLib,
  decodeValue,
  encodeArgs,
  encodeValue,
} from "#bffi";
import { ptr } from "bun:ffi";

// ---------------------------------------------------------------------------
// wire codec
// ---------------------------------------------------------------------------

describe("wire codec", () => {
  test("decodes the rust-side vector bytes", () => {
    // Vectors mirror bffi-async/src/value.rs tests (little-endian).
    expect(decodeValue(new Uint8Array([TAG_UNIT]))).toBeUndefined();
    expect(decodeValue(new Uint8Array([TAG_I32, 0xfe, 0xff, 0xff, 0xff]))).toBe(-2);
    expect(
      decodeValue(new Uint8Array([TAG_I64, 1, 0, 0, 0, 0, 0, 0, 0])),
    ).toBe(1n);
    expect(
      decodeValue(new Uint8Array([TAG_F64, 0, 0, 0, 0, 0, 0, 0xf8, 0x3f])),
    ).toBe(1.5);
    expect(decodeValue(new Uint8Array([TAG_BOOL, 1]))).toBe(true);
    expect(decodeValue(new Uint8Array([TAG_BOOL, 0]))).toBe(false);
    expect(decodeValue(new Uint8Array([TAG_STR, 3, 0, 0, 0, 0x68, 0x65, 0x79]))).toBe("hey");
    expect(decodeValue(new Uint8Array([TAG_BYTES, 2, 0, 0, 0, 9, 8]))).toEqual(
      new Uint8Array([9, 8]),
    );
  });

  test("encode/decode round-trips every value kind", () => {
    const values: Parameters<typeof encodeValue>[1][] = [
      undefined,
      -2,
      1.5,
      9007199254740993n,
      true,
      false,
      "hey \u{1F600}",
      new Uint8Array([9, 8]),
    ];
    for (const value of values) {
      const out: number[] = [];
      encodeValue(out, value);
      expect(decodeValue(new Uint8Array(out))).toEqual(value);
    }
  });

  test("encodeArgs concatenates records; empty args are a unit record", () => {
    expect([...encodeArgs([])]).toEqual([TAG_UNIT]);
    const bytes = encodeArgs([1, true]);
    expect(decodeValue(bytes.subarray(0, 5))).toBe(1);
    expect(decodeValue(bytes.subarray(5))).toBe(true);
  });

  test("rejects out-of-range values, unknown tags and truncated payloads", () => {
    expect(() => encodeArgs([18446744073709551616n])).toThrow(/out of range/);
    expect(() => encodeArgs([-1n])).toThrow(/negative bigint/);
    expect(() => decodeValue(new Uint8Array([255]))).toThrow(/unknown wire value tag/);
    expect(() => decodeValue(new Uint8Array())).toThrow(/truncated wire payload/);
  });
});

// ---------------------------------------------------------------------------
// declaration building
// ---------------------------------------------------------------------------

const FIXTURE = {
  bffi: 1,
  module: "matrix",
  functions: [
    {
      name: "add",
      export: "bffi_add",
      docs: ["Adds two numbers."],
      params: [
        { name: "a", ts: "number", abi: "u32" },
        { name: "b", ts: "number", abi: "u32" },
      ],
      ret: { ts: "number", abi: "u32" },
      out: "u32",
    },
    {
      name: "shout",
      export: "bffi_shout",
      docs: [],
      params: [{ name: "name", ts: "string", abi: "cstring" }],
      ret: { ts: "string", abi: "buffer" },
      out: "handle",
    },
    {
      name: "echo",
      export: "bffi_echo",
      docs: [],
      params: [{ name: "data", ts: "Uint8Array", abi: "ptr_len" }],
      ret: { ts: "Uint8Array", abi: "buffer" },
      out: "handle",
    },
    {
      name: "touch",
      export: "bffi_touch",
      docs: [],
      params: [{ name: "flag", ts: "boolean", abi: "bool" }],
      ret: { ts: "void", abi: "void" },
    },
  ],
  classes: [
    {
      name: "counter",
      release: "bffi_counter_release",
      docs: [],
      constructor: {
        name: "constructor",
        export: "bffi_counter_new",
        docs: [],
        params: [{ name: "start", ts: "number", abi: "u32" }],
        ret: { ts: "bigint", abi: "handle" },
        out: "handle",
      },
      fields: [
        {
          name: "value",
          export: "bffi_counter_value_get",
          docs: [],
          ts: "number",
          out: "u32",
        },
      ],
      methods: [
        {
          name: "increment",
          export: "bffi_counter_increment",
          docs: [],
          params: [],
          ret: { ts: "number", abi: "u32" },
          out: "u32",
        },
      ],
    },
  ],
} as const satisfies ModuleJson;

describe("buildDeclarations", () => {
  test("maps abi names onto bun:ffi spellings", () => {
    const declarations = buildDeclarations(FIXTURE);
    expect(declarations.bffi_add).toEqual({ args: ["u32", "u32", "pointer"], returns: "u32" });
    expect(declarations.bffi_shout).toEqual({ args: ["cstring", "pointer"], returns: "u32" });
    expect(declarations.bffi_echo).toEqual({ args: ["ptr", "u64", "pointer"], returns: "u32" });
    // bool crosses as u8; unit returns have no out-parameter.
    expect(declarations.bffi_touch).toEqual({ args: ["u8"], returns: "u32" });
    // Class members receive the instance handle first.
    expect(declarations.bffi_counter_new).toEqual({ args: ["u32", "pointer"], returns: "u32" });
    expect(declarations.bffi_counter_value_get).toEqual({
      args: ["u64", "pointer"],
      returns: "u32",
    });
    expect(declarations.bffi_counter_increment).toEqual({
      args: ["u64", "pointer"],
      returns: "u32",
    });
    expect(declarations.bffi_counter_release).toEqual({ args: ["u64"], returns: "u32" });
  });

  test("rejects unknown schema versions and duplicate exports", () => {
    expect(() => buildDeclarations({ ...FIXTURE, bffi: 2 })).toThrow(/unsupported bffi loader schema/);
    const dupe = {
      ...FIXTURE,
      functions: [
        ...FIXTURE.functions,
        { ...FIXTURE.functions[0], name: "again", export: "bffi_add" },
      ],
    } satisfies ModuleJson;
    expect(() => buildDeclarations(dupe)).toThrow(/duplicate export symbol/);
  });
});

// ---------------------------------------------------------------------------
// API factory over a mock library
// ---------------------------------------------------------------------------

/** A mock dlopen table implementing the FIXTURE ABI in plain JS.
 * Byte payloads hand out REAL pointers (`bun:ffi ptr`) so the
 * `readPointer` path is exercised end-to-end. The mock is typed
 * concretely; the `FfiLib` cast happens at the single call site. */
function mockLib() {
  const payloads: { bytes: Uint8Array; pointer: number }[] = [];
  const counter = { value: 0 };
  let alive = false;
  const store = (bytes: Uint8Array): bigint => {
    payloads.length = 0;
    payloads.push({ bytes, pointer: ptr(bytes) });
    return 1n;
  };
  return {
    bffi_error_take_last: () => null,
    bffi_error_name: () => 0,
    bffi_error_message_len: () => 0,
    bffi_error_message_ptr: () => null,
    bffi_error_cause_len: () => 0,
    bffi_error_cause_ptr: () => null,
    bffi_error_free: () => 0,
    bffi_buffer_length: () => payloads[0]?.bytes.length ?? 0,
    bffi_buffer: () => payloads[0]?.pointer ?? null,
    bffi_types_free: () => 0,
    bffi_add: (a: number, b: number, out: Uint32Array) => {
      out[0] = (a as number) + (b as number);
      return 0;
    },
    bffi_shout: (_name: string, out: BigUint64Array) => {
      out[0] = store(new TextEncoder().encode("HELLO!"));
      return 0;
    },
    bffi_echo: (data: unknown, len: bigint, out: BigUint64Array) => {
      void data;
      out[0] = store(new Uint8Array(Number(len)));
      return 0;
    },
    bffi_touch: (flag: number) => {
      if (flag !== 0) {
        return 11; // InvalidArgument: surfaces as a thrown Error.
      }
      return 0;
    },
    bffi_counter_new: (start: number, out: BigUint64Array) => {
      counter.value = start as number;
      alive = true;
      out[0] = 7n;
      return 0;
    },
    bffi_counter_value_get: (handle: bigint, out: Uint32Array) => {
      expect(handle).toBe(7n);
      out[0] = counter.value;
      return 0;
    },
    bffi_counter_increment: (handle: bigint, out: Uint32Array) => {
      expect(handle).toBe(7n);
      counter.value += 1;
      out[0] = counter.value;
      return 0;
    },
    bffi_counter_release: () => {
      expect(alive).toBe(true);
      alive = false;
      return 0;
    },
  };
}

/** The concrete mock meets the FfiLib boundary through one explicit
 * cast (bun:ffi symbol tables are untyped at this seam). */
const asLib = (lib: object): import("#bffi/runtime/error.ts").FfiLib =>
  lib as unknown as import("#bffi/runtime/error.ts").FfiLib;

describe("createApiFromLib", () => {
  const lib = asLib(mockLib());
  const api = createApiFromLib(FIXTURE, lib);

  test("decodes primitive outs into JS values", () => {
    expect(api.add(2, 3)).toBe(5);
  });

  test("decodes buffer returns into strings", () => {
    expect(api.shout("yo")).toBe("HELLO!");
  });

  test("encodes ptr_len parameters with a null pointer when empty", () => {
    const empty = api.echo(new Uint8Array(0));
    expect(empty).toEqual(new Uint8Array(0));
  });

  test("throws the drained error on a non-zero status", () => {
    expect(() => api.touch(true)).toThrow("bffi_touch failed: 11");
  });

  test("classes wrap the instance handle and release explicitly", () => {
    const c = new api.counter(10);
    expect(c.value).toBe(10);
    expect(c.increment()).toBe(11);
    expect(c.value).toBe(11);
    c.release();
    // A released instance must not release twice through GC.
    expect(() => new api.counter(0).release()).not.toThrow();
  });
});

// ---------------------------------------------------------------------------
// ABI handshake (loader manifest <-> binary runtime ABI)
// ---------------------------------------------------------------------------

describe("loader schema handshake fields", () => {
  test("BFFI_ABI_VERSION is the runtime ABI version", () => {
    expect(BFFI_ABI_VERSION).toBe(1);
  });

  test("accepts legacy manifests and well-typed abiVersion/exportsHash", () => {
    expect(() => assertSchema(FIXTURE)).not.toThrow();
    expect(() =>
      assertSchema({ ...FIXTURE, abiVersion: 1, exportsHash: "18339265623074793201" }),
    ).not.toThrow();
  });

  test("rejects mistyped abiVersion/exportsHash", () => {
    expect(() =>
      assertSchema({ ...FIXTURE, abiVersion: "1" as unknown as ModuleJson["abiVersion"] }),
    ).toThrow(/abiVersion/);
    expect(() =>
      assertSchema({ ...FIXTURE, exportsHash: 12 as unknown as ModuleJson["exportsHash"] }),
    ).toThrow(/exportsHash/);
  });

  test("buildDeclarations always declares the handshake symbols", () => {
    const declarations = buildDeclarations(FIXTURE);
    expect(declarations.bffi_runtime_abi_version).toEqual({ args: [], returns: "u32" });
    expect(declarations.bffi_module_exports_hash).toEqual({ args: [], returns: "u64" });
  });
});

describe("createApiFromLib ABI handshake", () => {
  const HASH = "18339265623074793201";

  test("legacy manifests without the new fields load", () => {
    expect(() => createApiFromLib(FIXTURE, asLib(mockLib()))).not.toThrow();
  });

  test("a manifest abiVersion mismatch throws before any wrapper is built", () => {
    expect(() => createApiFromLib({ ...FIXTURE, abiVersion: 2 }, asLib(mockLib()))).toThrow(
      "ABI version mismatch: manifest 2 != runtime 1 - rebuild the module " +
        "(bun bffi build) and regenerate api.gen.ts",
    );
  });

  test("a binary abi version mismatch throws", () => {
    const lib = { ...mockLib(), bffi_runtime_abi_version: () => 7 };
    expect(() => createApiFromLib(FIXTURE, asLib(lib))).toThrow(
      "ABI version mismatch: binary 7 != runtime 1 - rebuild the module (bun bffi build)",
    );
  });

  test("an exports hash mismatch throws (stale manifest vs binary)", () => {
    const lib = { ...mockLib(), bffi_module_exports_hash: () => 999n };
    expect(() =>
      createApiFromLib({ ...FIXTURE, exportsHash: HASH }, asLib(lib)),
    ).toThrow(
      "exports hash mismatch: the loader manifest does not describe this binary " +
        "(stale .bffi/bffi.api.json vs the built cdylib)",
    );
  });

  test("exportsHash with a missing binary symbol throws", () => {
    expect(() =>
      createApiFromLib({ ...FIXTURE, exportsHash: HASH }, asLib(mockLib())),
    ).toThrow("binary predates the ABI handshake exports - rebuild with the current bffi version");
  });

  test("a matching exports hash passes the handshake", () => {
    const lib = {
      ...mockLib(),
      bffi_runtime_abi_version: () => 1,
      bffi_module_exports_hash: () => BigInt(HASH),
    };
    expect(() =>
      createApiFromLib({ ...FIXTURE, abiVersion: 1, exportsHash: HASH }, asLib(lib)),
    ).not.toThrow();
  });
});
