# Part 01 - Workspace scaffold

No maths yet. This part fixes the structure that every later part builds on.

## Crate split

`pricing` is pure maths with no internal dependencies, so it can be tested and benchmarked alone. `domain` holds shared types and uses `pricing`. `storage` and `portfolio` sit on `domain`. `marketdata` uses `domain` and `storage`. `risk` uses `portfolio`. `engine` joins them all and `server` is the only HTTP layer. The graph has no cycles, and nothing below `server` knows about HTTP, so the engine can be tested without a network. `server` has both a library and a binary so integration tests can build the router.

## Lint choices

- `unsafe_code = "forbid"`, since nothing here needs it.
- Clippy denies `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented` and `dbg_macro`. This enforces "no panics" mechanically. `clippy.toml` relaxes unwrap and expect inside tests only.
- `pedantic` is a warning at priority -1, so the explicit denies win. Five noisy pedantic lints are allowed because numeric code trips them constantly: `cast_precision_loss`, `similar_names`, `many_single_char_names`, `module_name_repetitions`, `must_use_candidate`.
- `make check` runs clippy with `-D warnings`, so warnings fail the build.

## Going back to any part

Each part ends with a commit, a tag `part-NN` and a snapshot. To inspect a part, run `git checkout part-NN` (then `git checkout main` to return), or unpack the archive from `../snapshots/`. `make snapshot` refuses to run on a dirty tree, so every archive matches a commit. History is never rewritten and tags are never moved. Re-runs get `part-NN-r2` and so on.

## Interview questions

- **Why split `pricing` from `domain`?** Pricing takes plain `f64` inputs and has no I/O, so its accuracy can be verified against reference values and property tests without any application types.
- **Why deny `unwrap` at the lint level?** A review comment can be missed. A compile error cannot, and it forces typed errors at every boundary.
- **Why a lib and a bin in `server`?** Integration tests can construct the axum router directly instead of spawning a process.
