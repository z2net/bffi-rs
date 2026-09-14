/**
 * Hardening tests for the JS wire decoder: bounds on declared
 * lengths/counts, the configurable payload ceiling and the nesting
 * depth limit (mirrors the Rust-side MAX_WIRE_PAYLOAD / MAX_WIRE_DEPTH).
 *
 * Run with `bun test packages/bffi`.
 */
import { afterEach, describe, expect, test } from "bun:test";

import {
  MAX_WIRE_DEPTH,
  MAX_WIRE_PAYLOAD,
  decodeValue,
  encodeValue,
  setMaxWirePayload,
  type WireValue,
} from "../src/index.ts";
import {
  TAG_BYTES,
  TAG_ERROR,
  TAG_RECORD,
  TAG_SEQ,
  TAG_STR,
  TAG_UNIT,
  decodeErrorEnvelope,
} from "../src/runtime/wire.ts";

/** u32 LE bytes for `n`. */
function u32(n: number): number[] {
  return [n & 0xff, (n >>> 8) & 0xff, (n >>> 16) & 0xff, (n >>> 24) & 0xff];
}

describe("wire decoder hardening", () => {
  afterEach(() => {
    setMaxWirePayload(MAX_WIRE_PAYLOAD);
  });

  test("truncated length field throws the wire truncation error", () => {
    // [TAG_STR][len: only 1 of 4 bytes present]
    expect(() => decodeValue(new Uint8Array([TAG_STR, 5]))).toThrow(
      /wire payload truncated: declared 5 bytes at offset 0, have 2/,
    );
  });

  test("declared length beyond the buffer throws before slicing", () => {
    const str = new Uint8Array([TAG_STR, ...u32(100), 0x68, 0x65, 0x79]);
    expect(() => decodeValue(str)).toThrow(
      /wire payload truncated: declared 100 bytes at offset 0, have 8/,
    );

    const bytes = new Uint8Array([TAG_BYTES, ...u32(9), 1]);
    expect(() => decodeValue(bytes)).toThrow(
      /wire payload truncated: declared 9 bytes/,
    );

    const error = new Uint8Array([TAG_ERROR, ...u32(1 << 20)]);
    expect(() => decodeValue(error)).toThrow(
      /wire payload truncated: declared 1048576 bytes/,
    );
  });

  test("record/seq headers and counts are bounded", () => {
    expect(() => decodeValue(new Uint8Array([TAG_RECORD, 3, 0, 0]))).toThrow(
      /wire payload truncated: declared 5 bytes at offset 0, have 4/,
    );
    expect(() => decodeValue(new Uint8Array([TAG_SEQ, 3, 0, 0]))).toThrow(
      /wire payload truncated: declared 5 bytes at offset 0, have 4/,
    );
    const seq = new Uint8Array([TAG_SEQ, ...u32(0x7fffffff)]);
    expect(() => decodeValue(seq)).toThrow(
      /wire payload too large: declared 2147483647 items at offset 0, limit is 67108864/,
    );
  });

  test("declared length above the default maximum throws", () => {
    expect(MAX_WIRE_PAYLOAD).toBe(64 * 1024 * 1024);
    const bytes = new Uint8Array([TAG_STR, ...u32(MAX_WIRE_PAYLOAD + 1)]);
    expect(() => decodeValue(bytes)).toThrow(
      /wire payload too large: declared 67108865 bytes at offset 0, limit is 67108864/,
    );
  });

  test("setMaxWirePayload enforces a custom limit and restores", () => {
    setMaxWirePayload(16);
    const bytes = new Uint8Array([TAG_STR, ...u32(20), ...new Uint8Array(20)]);
    expect(() => decodeValue(bytes)).toThrow(
      /wire payload too large: declared 20 bytes at offset 0, limit is 16/,
    );
    setMaxWirePayload(MAX_WIRE_PAYLOAD);
    expect(
      decodeValue(new Uint8Array([TAG_STR, ...u32(20), ...new Uint8Array(20)])),
    ).toBe("\0".repeat(20));
  });

  test("setMaxWirePayload rejects invalid limits", () => {
    for (const bad of [0, -1, 1.5, Number.NaN, 0x1_0000_0000]) {
      expect(() => setMaxWirePayload(bad)).toThrow(/invalid max wire payload/);
    }
  });

  test("nesting deeper than 64 throws; 64 passes", () => {
    const nest = (depth: number): Uint8Array => {
      const out: number[] = [];
      for (let i = 0; i < depth; i++) {
        out.push(TAG_RECORD, ...u32(1));
      }
      out.push(TAG_UNIT);
      return new Uint8Array(out);
    };
    expect(MAX_WIRE_DEPTH).toBe(64);

    let expected: WireValue = undefined;
    for (let i = 0; i < 64; i++) {
      expected = { fields: [expected] };
    }
    expect(decodeValue(nest(64))).toEqual(expected);
    expect(() => decodeValue(nest(65))).toThrow(/wire nesting deeper than 64/);
  });

  test("error envelope strings and payload records are bounded", () => {
    const truncatedMessage = new Uint8Array([
      TAG_ERROR, ...u32(13), ...u32(1), 0x56, ...u32(0x10_0000),
    ]);
    expect(() => decodeErrorEnvelope(truncatedMessage, 0)).toThrow(
      /wire payload truncated: declared 1048576 bytes at offset 10, have 14/,
    );

    const hugeMessage = new Uint8Array([
      TAG_ERROR, ...u32(13), ...u32(1), 0x56, ...u32(0xffffffff),
    ]);
    expect(() => decodeErrorEnvelope(hugeMessage, 0)).toThrow(
      /wire payload too large: declared 4294967295 bytes at offset 10, limit is 67108864/,
    );

    const hugeRecord = new Uint8Array([
      TAG_ERROR, ...u32(13), ...u32(0), ...u32(0), TAG_RECORD, ...u32(0xffffffff),
    ]);
    expect(() => decodeErrorEnvelope(hugeRecord, 0)).toThrow(
      /wire payload too large: declared 4294967295 items at offset 13, limit is 67108864/,
    );
  });

  test("well-formed payloads still decode (envelope + values)", () => {
    const out: number[] = [];
    encodeValue(out, "hey");
    expect(decodeValue(new Uint8Array(out))).toBe("hey");

    const envelope = new Uint8Array([
      TAG_ERROR, ...u32(13), ...u32(1), 0x56, ...u32(2), 0x6f, 0x6b, TAG_UNIT,
    ]);
    const { envelope: decoded } = decodeErrorEnvelope(envelope, 0);
    expect(decoded).toEqual({ code: 13, variant: "V", message: "ok", payload: null });
  });
});
