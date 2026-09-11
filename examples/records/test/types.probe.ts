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
