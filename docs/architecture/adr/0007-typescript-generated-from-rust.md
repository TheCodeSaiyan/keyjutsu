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
