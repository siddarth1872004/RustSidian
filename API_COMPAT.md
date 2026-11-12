# Obsidian API compatibility

**Nothing is implemented.** There is no JavaScript runtime in this build, so every export of
`obsidian.d.ts` is **Unsupported**. A per-export table would be several hundred identical rows and
is omitted until `compat-js` exists; it must be generated from the typings at that point, not
written by hand.

What was done: a spike comparing JS engines (DECISIONS.md §3). `boa_engine` evaluated a
plugin-shaped bundle correctly; `rquickjs` and `deno_core` could not be built on the development
machine. No CodeMirror 6 adapter exists.
