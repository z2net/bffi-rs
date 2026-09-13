# records example

The B1 composite types end to end: records (`#[derive(BffiRecord)]`
structs), unit enums (`#[derive(BffiEnum)]`) and `Vec<T>` sequences
crossing the boundary as wire-encoded transient buffers - through the
full `@z2net/bffi` pipeline (cargo build -> loader JSON -> api.gen ->
dlopen).

## Run

From the repo root:

```sh
bun test examples/records
```

The test builds the cdylib, generates the typed API and asserts the
round-trips: a record built from primitives, record parameters and
returns, sequences in and numbers/strings out, the enum result
channel, its domain error, and the strict composite kind checks.
