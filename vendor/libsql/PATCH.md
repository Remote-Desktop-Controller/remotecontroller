# Local libSQL 0.9.30 ownership patch

Source: crates.io `libsql` 0.9.30, copied without registry markers or its
standalone lockfile. Production dependencies/features are unchanged. License:
`LICENSE.md` (MIT, the sqld authors).
Original crate SHA-256: `30fe980ac5693ed1f3db490559fb578885e913a018df64af8a1a46e1959a78df`.

The local connection wrapper and its inner connection both call `disconnect`
on drop. The unpatched method closes the same sqlite3 pointer twice. This is a
use-after-free, even on operating systems where allocator behavior hides it.

- Upstream issue: https://github.com/tursodatabase/libsql/issues/2251
- Proposed upstream fix: https://github.com/tursodatabase/libsql/pull/2261
- Patch: clear `raw` after `sqlite3_close_v2` in `src/local/connection.rs`.
- Added regression: `disconnect_is_idempotent_and_clears_closed_handle` checks
  the closed handle before a second disconnect, so the unpatched code fails
  deterministically without requiring an allocator-specific native crash.

Run `pwsh -File tools/test-libsql-patch.ps1` from the repository. The temporary
test manifest omits only upstream Criterion/pprof dev dependencies (profiling
is unrelated to the native ownership test); all production source is compiled
directly from this directory. Runtime tests and benchmarks exercise this same
patch through `[patch.crates-io]` and the workspace lockfile.

Remove this override only after an upstream published version contains the fix
and the ownership regression, release storage tests and benchmarks pass.
