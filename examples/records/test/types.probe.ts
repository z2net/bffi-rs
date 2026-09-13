/** Type-level probe for the B1 named-type resolution (run via
 * `bunx tsc --noEmit` - errors here are the signal). */
import type { ApiOf, ModuleJson } from "@z2net/bffi";

const probe = {
  bffi: 1,
  module: "probe",
  functions: [
    {
      name: "classify",
      export: "bffi_classify",
      docs: [],
      params: [{ name: "samples", ts: "ProbeSample[]", abi: "ptr_len" }],
      ret: { ts: "ProbeAxis", abi: "buffer" },
      out: "handle",
    },
    {
      name: "make",
      export: "bffi_make",
      docs: [],
      params: [{ name: "axis", ts: "ProbeAxis", abi: "ptr_len" }],
      ret: { ts: "ProbeSample", abi: "buffer" },
      out: "handle",
    },
  ],
  classes: [],
  records: [
    {
      name: "ProbeSample",
      docs: [],
      fields: [
        { name: "at", docs: [], ts: "number" },
        { name: "axis", docs: [], ts: "ProbeAxis" },
      ],
    },
  ],
  enums: [
    {
      name: "ProbeAxis",
      docs: [],
      variants: [
        { name: "Horizontal", docs: [] },
        { name: "Vertical", docs: [] },
      ],
    },
  ],
} as const satisfies ModuleJson;

type ProbeApi = ApiOf<typeof probe>;

// The enum parameter must be the variant union, not the bare literal.
declare const vertical: "Vertical";
const _axisArg: Parameters<ProbeApi["make"]>[0] = vertical;
const _ret: ReturnType<ProbeApi["classify"]> = "Horizontal";
void _axisArg;
void _ret;

// The record field type must resolve too (axis inside ProbeSample).
declare const sample: ReturnType<ProbeApi["make"]>;
const _field: string = sample.axis;
void _field;

// The Option-field ts flavors: a `X | null` field must resolve to
// the nullable JS type (not the literal name), flat and named.
const nullableProbe = {
  bffi: 1,
  module: "nullableProbe",
  functions: [
    {
      name: "echo",
      export: "bffi_echo",
      docs: [],
      params: [{ name: "p", ts: "ProbeProfile", abi: "ptr_len" }],
      ret: { ts: "ProbeProfile", abi: "buffer" },
      out: "handle",
    },
  ],
  classes: [],
  records: [
    {
      name: "ProbeProfile",
      docs: [],
      fields: [
        { name: "level", docs: [], ts: "number | null" },
        { name: "rank", docs: [], ts: "bigint | null" },
        { name: "muted", docs: [], ts: "boolean | null" },
        { name: "nick", docs: [], ts: "string | null" },
        { name: "home", docs: [], ts: "ProbeSample | null" },
      ],
    },
    {
      name: "ProbeSample",
      docs: [],
      fields: [
        { name: "at", docs: [], ts: "number" },
        { name: "axis", docs: [], ts: "ProbeAxis" },
      ],
    },
  ],
  enums: [
    {
      name: "ProbeAxis",
      docs: [],
      variants: [
        { name: "Horizontal", docs: [] },
        { name: "Vertical", docs: [] },
      ],
    },
  ],
} as const satisfies ModuleJson;

type NullableApi = ApiOf<typeof nullableProbe>;
declare const profile: Parameters<NullableApi["echo"]>[0];
const _level: number | null = profile.level;
const _rank: bigint | null = profile.rank;
const _muted: boolean | null = profile.muted;
const _nick: string | null = profile.nick;
const _home: { at: number; axis: "Horizontal" | "Vertical" } | null = profile.home;
void _level;
void _rank;
void _muted;
void _nick;
void _home;
