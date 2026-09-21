/**
 * The per-library disposal registry: JS-side resources whose native
 * counterparts must be retired deterministically (JS-bound callback
 * trampolines, stream wakes) register a closer here, and
 * {@link makeLibDisposer} - attached to every `Api` as
 * `[Symbol.dispose]` - runs all of them. This is the JS half of the
 * `using` contract (Bun 1.4 executes it natively): `await using api =
 * await bffi({...})` leaves no JSCallback trampoline behind.
 */
import type { FfiLib } from "#bffi/runtime/error.ts";

/** The closer sets, keyed by the dlopen'ed symbol table object. */
const REGISTRY = new WeakMap<FfiLib, Set<() => void>>();

/** Registers a closer for `lib`; returns the live set (removing the
 * closer from it unregisters - an explicitly revoked callback no
 * longer needs the Api-level dispose). */
export function registerDisposer(lib: FfiLib, closer: () => void): Set<() => void> {
  let set = REGISTRY.get(lib);
  if (set === undefined) {
    set = new Set();
    REGISTRY.set(lib, set);
  }
  set.add(closer);
  return set;
}

/** Runs every registered closer for `lib` and clears the set.
 * Idempotent per close: the closers themselves are idempotent. */
export function disposeLib(lib: FfiLib): void {
  const set = REGISTRY.get(lib);
  if (set === undefined) {
    return;
  }
  REGISTRY.delete(lib);
  for (const closer of set) {
    closer();
  }
}

/**
 * The `[Symbol.dispose]` implementation every Api carries: retires
 * the JS-side trampolines created against this library. The dlopen'ed
 * library itself stays loaded (bun:ffi has no dlclose) - dispose
 * claims the CALLBACK surface, not the code.
 */
export function makeLibDisposer(lib: FfiLib): { [Symbol.dispose](): void } {
  return {
    [Symbol.dispose](): void {
      disposeLib(lib);
    },
  };
}

/** The memory-pressure listener: a synchronous GC pass drives the
 * registry finalizers. Captures nothing, so it lives at module scope
 * and `process.off` always receives the exact reference `process.on`
 * got. */
const memoryPressureListener = (level: "warning" | "critical"): void => {
  void level;
  // A synchronous collection runs the registry finalizers: native
  // handles behind unreachable wrappers are released now.
  Bun.gc(true);
};

/**
 * Installs the Bun 1.4 memory-pressure hook: when the OS signals low
 * memory (`process.on("memoryPressure")`), a synchronous GC pass
 * drives the FinalizationRegistry finalizers, releasing native class
 * handles and stream drops EARLY instead of waiting for the next
 * natural collection. Returns the uninstaller. Idempotent to install.
 */
export function installMemoryPressureGC(): () => void {
  if (installed) {
    return () => {};
  }
  installed = true;
  process.on("memoryPressure", memoryPressureListener);
  return () => {
    process.off("memoryPressure", memoryPressureListener);
    installed = false;
  };
}

let installed = false;
