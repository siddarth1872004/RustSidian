# Plugin SDK

**Not implemented.** There is no WASM plugin host, no WIT interface, no SDK crate and no
`walnut plugin` scaffold (`walnut plugin …` exits with an explanatory error).

Decisions already made for when it is built (DECISIONS.md §5): `wasmi` as the runtime for its
size and fuel metering, with `wasmtime` to be compared once a C toolchain is available.
