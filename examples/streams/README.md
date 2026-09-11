# streams example

The B2 stream surface end to end: `#[bffi_stream]` exports hand
JavaScript an `AsyncIterableIterator` over a Rust `Iterator`
(pull-chunk through the generic `bffi_stream_next` export) - via
the full `@z2net/bffi` pipeline (cargo build -> loader JSON ->
api.gen -> dlopen).

## Run

From the repo root:

```sh
bun test examples/streams
```

The suite covers numeric streams, record streams (B1 composites
compose), string transforms, the early-break release path, and the
exact generated types.
