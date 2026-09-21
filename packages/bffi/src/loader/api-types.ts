/**
 * The type-level mapping of the loader schema: from a `ModuleJson`
 * literal (and its `ts` names) to the exact TypeScript signatures the
 * runtime factory delivers - `TsOf` per name, `ParamsOf`/`FnOf` per
 * descriptor, `ApiOf`/`ClassOf` for the whole module.
 *
 * Split out of `api.ts` (the runtime factory): this file is types
 * only - it compiles away and never reaches the generated module's
 * runtime.
 */

import type { FunctionJson, ModuleJson, TsName } from "./loader.ts";

/** The record table entry named `N` (`never` when absent). */
type NamedRecord<M extends ModuleJson, N extends string> = Extract<
  NonNullable<M["records"]>[number],
  { name: N }
>;

/** The enum table entry named `N` (`never` when absent). */
type NamedEnum<M extends ModuleJson, N extends string> = Extract<
  NonNullable<M["enums"]>[number],
  { name: N }
>;

/** The object shape of one record entry: fields keyed by name (the
 * module context threads through - a record field may reference
 * another composite of the same module). */
type RecordShape<R extends { fields: { name: string; ts: TsName }[] }, M extends ModuleJson> = {
  [F in R["fields"][number] as F["name"]]: TsOf<F["ts"], M>;
};

/** One variant entry as embedded in the schema literal (the payload
 * `fields` key is written only for variants that carry a payload). */
type SchemaVariant = {
  name: string;
  fields?: readonly { name: string; ts: TsName }[];
};

/** The payload object of one variant's fields, resolved through
 * `TsOf` (the inferred tuple keeps the positional order; the mapped
 * type remaps the numeric indices onto the field names - array-level
 * keys like `length` or the number index are remapped away). */
type VariantShape<F, M extends ModuleJson> = F extends readonly {
  name: string;
  ts: TsName;
}[]
  ? {
      [K in keyof F as K extends `${number}`
        ? F[K] extends { name: string }
          ? F[K]["name"]
          : never
        : never]: F[K] extends { ts: TsName } ? TsOf<F[K]["ts"], M> : never;
    }
  : never;

/** One member of a payload-carrying enum's union: every variant
 * (unit ones included) renders as a uniform `{ kind, ... }` object. */
type DiscriminatedVariant<V, M extends ModuleJson> = V extends unknown
  ? V extends { name: infer N; fields: infer F }
    ? { kind: N & string } & VariantShape<F, M>
    : V extends { name: infer N }
      ? { kind: N & string }
      : never
  : never;

/** The union of one enum entry's variants: a plain string-literal
 * union for unit-only enums (v1 compatibility - no variant carries
 * the `fields` key), a discriminated union of `{ kind, ... }`
 * objects once any variant carries a payload. */
type EnumUnion<E extends { variants: readonly SchemaVariant[] }, M extends ModuleJson> =
  E["variants"][number] extends { fields?: undefined }
    ? E["variants"][number]["name"]
    : DiscriminatedVariant<E["variants"][number], M>;

/**
 * The TypeScript type for one `ts` name in the schema. The Promise
 * cases are spelled out (a closed table, no template-literal infer):
 * the schema names them exactly. The B1 composites resolve against
 * the module's own `records`/`enums` tables: a record maps to its
 * field shape, an enum to the union of its variant names, and the
 * array forms wrap their element type. An unknown name falls back to
 * its own string literal (the validator rejects those upfront).
 */
export type TsOf<S extends TsName, M extends ModuleJson = ModuleJson> =
  S extends "Promise<void>"
    ? Promise<void>
    : S extends "Promise<number>"
      ? Promise<number>
      : S extends "Promise<bigint>"
        ? Promise<bigint>
        : S extends "Promise<boolean>"
          ? Promise<boolean>
          : S extends "Promise<string>"
            ? Promise<string>
             : S extends "Promise<Uint8Array>"
               ? Promise<Uint8Array>
               : S extends "Promise<number[]>"
                 ? Promise<number[]>
                 : S extends "Promise<bigint[]>"
                   ? Promise<bigint[]>
                   : S extends "Promise<boolean[]>"
                     ? Promise<boolean[]>
                     : S extends "Promise<string[]>"
                       ? Promise<string[]>
                         : S extends "Promise<Uint8Array[]>"
                           ? Promise<Uint8Array[]>
                           : S extends "Promise<string | null>"
                             ? Promise<string | null>
                             : S extends "Promise<Uint8Array | null>"
                               ? Promise<Uint8Array | null>
                               : S extends "Promise<number[] | null>"
                                 ? Promise<number[] | null>
                                 : S extends "Promise<bigint[] | null>"
                                   ? Promise<bigint[] | null>
                                   : S extends "Promise<boolean[] | null>"
                                     ? Promise<boolean[] | null>
                                     : S extends "Promise<string[] | null>"
                                       ? Promise<string[] | null>
                                       : S extends "Promise<Uint8Array[] | null>"
                                         ? Promise<Uint8Array[] | null>
                                         : S extends `Promise<${infer P} | null>`
                                           ? P extends TsName
                                             ? [NamedRecord<M, P>] extends [never]
                                               ? [NamedEnum<M, P>] extends [never]
                                                 ? S
                                                 : NamedEnum<M, P> extends infer E
                                                   ? E extends { variants: { name: string }[] }
                                                     ? Promise<EnumUnion<E, M> | null>
                                                     : S
                                                   : S
                                               : NamedRecord<M, P> extends infer Rec
                                                 ? Rec extends { fields: { name: string; ts: TsName }[] }
                                                   ? Promise<RecordShape<Rec, M> | null>
                                                   : S
                                                 : S
                                             : S
                                           : S extends `Promise<${infer P}>`
                           ? P extends TsName
                             ? [NamedRecord<M, P>] extends [never]
                               ? [NamedEnum<M, P>] extends [never]
                                 ? S
                                 : NamedEnum<M, P> extends infer E
                                   ? E extends { variants: { name: string }[] }
                                     ? Promise<EnumUnion<E, M>>
                                     : S
                                   : S
                               : NamedRecord<M, P> extends infer Rec
                                 ? Rec extends { fields: { name: string; ts: TsName }[] }
                                   ? Promise<RecordShape<Rec, M>>
                                   : S
                                 : S
                             : S
                            : S extends "string | null"
                 ? string | null
                 : S extends "Uint8Array | null"
                   ? Uint8Array | null
                   : S extends "number | null"
                     ? number | null
                     : S extends "bigint | null"
                       ? bigint | null
                       : S extends "boolean | null"
                         ? boolean | null
                         : S extends "number[]"
                    ? number[]
                    : S extends "bigint[]"
                      ? bigint[]
                      : S extends "boolean[]"
                        ? boolean[]
                  : S extends "string[]"
                    ? string[]
                    : S extends "Uint8Array[]"
                      ? Uint8Array[]
                      : S extends "number[] | null"
                        ? number[] | null
                        : S extends "bigint[] | null"
                          ? bigint[] | null
                          : S extends "boolean[] | null"
                            ? boolean[] | null
                            : S extends "string[] | null"
                              ? string[] | null
                              : S extends "Uint8Array[] | null"
                                ? Uint8Array[] | null
                                : S extends `${infer N} | null`
                                  ? N extends TsName
                                    ? [NamedRecord<M, N>] extends [never]
                                      ? [NamedEnum<M, N>] extends [never]
                                        ? S
                                        : NamedEnum<M, N> extends infer E
                                          ? E extends { variants: { name: string }[] }
                                            ? EnumUnion<E, M> | null
                                            : S
                                          : S
                                      : NamedRecord<M, N> extends infer Rec
                                        ? Rec extends { fields: { name: string; ts: TsName }[] }
                                          ? RecordShape<Rec, M> | null
                                          : S
                                        : S
                                    : S
                                  : S extends "AsyncIterableIterator<number>"
                            ? AsyncIterableIterator<number>
                            : S extends "AsyncIterableIterator<bigint>"
                              ? AsyncIterableIterator<bigint>
                              : S extends "AsyncIterableIterator<boolean>"
                                ? AsyncIterableIterator<boolean>
                                : S extends "AsyncIterableIterator<string>"
                                  ? AsyncIterableIterator<string>
                                  : S extends "AsyncIterableIterator<Uint8Array>"
                                    ? AsyncIterableIterator<Uint8Array>
                                    : S extends `AsyncIterableIterator<${infer N2} | Error>`
                                      ? N2 extends TsName
                                        ? AsyncIterableIterator<TsOf<N2, M> | Error>
                                        : S
                                      : S extends `AsyncIterableIterator<${infer N}>`
                                        ? N extends TsName
                                          ? AsyncIterableIterator<TsOf<N, M>>
                                          : S
                                        : S extends `${infer N}[]`
                            ? N extends TsName
                              ? TsOf<N, M>[]
                              : S
                            : [NamedRecord<M, S & string>] extends [never]
                              ? [NamedEnum<M, S & string>] extends [never]
                                ? S extends "void"
                                  ? void
                                  : S extends "number"
                                    ? number
                                    : S extends "bigint"
                                      ? bigint
                                      : S extends "boolean"
                                        ? boolean
                                        : S extends "string"
                                          ? string
                                          : S extends "Uint8Array"
                                            ? Uint8Array
                                            : S
                                : NamedEnum<M, S & string> extends infer E
                                  ? E extends { variants: { name: string }[] }
                                    ? EnumUnion<E, M>
                                    : S
                                  : S
                              : NamedRecord<M, S & string> extends infer Rec
                                ? Rec extends { fields: { name: string; ts: TsName }[] }
                                  ? RecordShape<Rec, M>
                                  : S
                                : S;

/** Maps a params array onto a positional tuple type. */
export type ParamsOf<J extends readonly { ts: TsName }[], M extends ModuleJson = ModuleJson> = {
  [K in keyof J]: J[K] extends { ts: infer T } ? (T extends TsName ? TsOf<T, M> : never) : never;
};

/** The callable signature of one function/method descriptor. */
export type FnOf<J extends FunctionJson, M extends ModuleJson = ModuleJson> = (
  ...args: ParamsOf<J["params"], M>
) => TsOf<J["ret"]["ts"], M>;

/** The typed API object for a module literal. */
export type ApiOf<J extends ModuleJson> = {
  [F in J["functions"][number] as F["name"]]: FnOf<F, J>;
} & {
  [C in J["classes"][number] as C["name"]]: ClassOf<C, J>;
};

/** The typed class constructor + instance shape for one class. */
export type ClassOf<C extends ModuleJson["classes"][number], M extends ModuleJson = ModuleJson> = {
  new (
    ...args: ParamsOf<C["constructor"]["params"], M>
  ): {
    readonly [F in C["fields"][number] as F["name"]]: TsOf<F["ts"], M>;
  } & {
    [Me in C["methods"][number] as Me["name"]]: FnOf<Me, M>;
  } & {
    /** Releases the native handle early (also runs on GC). */
    release(): void;
  };
};
