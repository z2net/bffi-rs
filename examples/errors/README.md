# errors example

The B3 typed error surface end to end: `#[derive(BffiError)]` enums
cross the boundary with their user code in the ABI status, the
variant name as JS `e.name` and the named fields as the `e.payload`
record - via the full `@z2net/bffi` pipeline (cargo build -> loader
JSON -> api.gen -> dlopen).

## Run

From the repo root:

```sh
bun test examples/errors
```

The suite covers successful calls, `NOT_FOUND` with the queried id
in the payload, `INVALID_AGE` with value and bound, the `String`
ad-hoc shortcut (DomainError 13) and explicit framework errors.
