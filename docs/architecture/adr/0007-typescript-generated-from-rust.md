# 0007: TypeScript IPC types are generated from Rust

Status: accepted, Milestone 0.

## Decision

Types that cross the IPC boundary derive `ts_rs::TS` and are written to
`packages/types/src/generated` whenever `cargo test` runs.
`packages/types/scripts/write-index.mjs` writes the index. CI regenerates both
and fails if the committed copies differ, so a Rust change that alters the
wire format cannot land without the TypeScript changing with it.

Types the desktop window sends or receives live in `keyjutsu-core::ipc`
rather than the desktop crate. The desktop crate embeds the built frontend at
compile time, so if its tests generated the types the frontend needs, neither
could be built first.

## Consequences

- A `u64` becomes a TypeScript `bigint`, which JSON cannot carry. The engine's
  seed is a `u32` for that reason; any future 64-bit field on the wire needs
  a `#[ts(type = "number")]` and a range check, or a string.
- TypeScript is pinned to `~6.0.3`. TypeScript 7 is current, but
  typescript-eslint 8.70 supports only `<6.1`. Revisit when it does.

## Addendum, Milestone 3: one folder per crate

ts-rs names each file after the Rust type, so two crates exporting a type of
the same name to the same folder overwrite each other, silently, and which
one survives depends on the order tests run in. The plan crate's `Check`
replaced the readiness `Check` the first time it was generated; the desktop's
typecheck caught it. The plan crate's `ExecutionMode` also replaced the
engine's, which nothing caught only because the two happen to be identical.

From Milestone 3, a crate exports to a folder of its own
(`#[ts(export, export_to = "plan/")]`), and each folder becomes a namespace in
`@keyjutsu/types` (`import type { plan } from "@keyjutsu/types"`, then
`plan.Check`). The crates written before this still export to the top level;
none of their names collide, and moving them would change every import for
no gain. New crates should follow the folder rule.
