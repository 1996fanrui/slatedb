# Enforcing `clippy::absolute_paths` in SlateDB — data for a decision

Status: **analysis only**, nothing committed. All numbers measured locally on
`community/main` (`d12891f1`) with clippy 1.91.0 on 2026-08-11.

## 1. Why

Two review comments on my PRs asked for the same thing — import the item instead of
spelling out its path inline:

| PR | File | Flagged code | Reviewer |
|----|------|--------------|----------|
| [#2006](https://github.com/slatedb/slatedb/pull/2006#discussion_r3754004177) | `slatedb/src/memtable_flusher/manifest_writer.rs` | `manifest: &slatedb_txn_obj::DirtyObject<crate::manifest::Manifest>` | criccomini — "Import these (`use ...`)." |
| [#1939](https://github.com/slatedb/slatedb/pull/1939) | `slatedb/src/db.rs` | `\|metadata: crate::mem_table::KVTableMetadata\|` | criccomini — "Nit mind `use` for this? Claude using fqns is a bit annoying all the time :P" |

Three distinct inline paths across those two comments:

| Path | Segments | Caught at `max-segments = 2` | Caught at `max-segments = 1` |
|------|----------|------------------------------|------------------------------|
| `crate::mem_table::KVTableMetadata` | 3 | yes | yes |
| `crate::manifest::Manifest` | 3 | yes | yes |
| `slatedb_txn_obj::DirtyObject` | 2 | **no** | yes |

This is mechanical and reviewers should not have to catch it by hand.

## 2. Mechanism (verified locally)

`clippy::absolute_paths` (restriction group, off by default) flags paths that resolve
through a crate root — i.e. starting with `crate`, `::`, or a crate name. Three moving
parts, and they cannot all live in one file:

| Concern | Location | Notes |
|---------|----------|-------|
| What counts as a violation | `.config/clippy.toml` | Single source; every crate symlinks `clippy.toml -> ../.config/clippy.toml`. A `clippy.toml` at the workspace root is **ignored** — clippy resolves config from the crate manifest directory (measured: no effect). |
| Turning the lint on | `[workspace.lints.clippy]` in the root `Cargo.toml`, or `#![warn(...)]` per crate root | `clippy.toml` cannot enable a lint. |
| Exempting test code | `#![cfg_attr(test, allow(clippy::absolute_paths))]` at each crate root | `clippy.toml` has no notion of `cfg`. Already the house idiom — `slatedb/src/lib.rs` does this for `unwrap_used`, `panic`, `disallowed_types`, `disallowed_methods`, `disallowed_macros`. |

CI needs **no workflow change**: `pr.yaml` already sets `RUSTFLAGS: "-Dwarnings"` and runs
`cargo hack clippy --each-feature --no-dev-deps --workspace` (production code, `cfg(test)`
off) plus `cargo hack clippy --workspace --tests` (test targets, `cfg(test)` on → exempt).
A `"warn"` level therefore fails CI on production code only.

Verified end to end by injecting one violation into an otherwise clean crate:

```
error: could not compile `slatedb-common` (lib) due to 1 previous error
  --> slatedb-common/src/clock.rs:21:24
   = note: `-D clippy::absolute-paths` implied by `-D warnings`
```

### Two limits of the config, both measured

- **`absolute-paths-allowed-crates` is crate-level only.** Adding `"object_store::Result"`
  or `"tokio::task"` changes nothing (963 → 963) and clippy emits no warning about the
  unusable entries. Per-symbol exemptions are only possible with `#[allow(...)]` at the use
  site, which is more verbose than the `use` it would avoid.
- **"Only flag our own crates" is not expressible.** It would require allow-listing every
  external crate — 445 packages in this workspace's graph.

### What the exemption does and does not cover

- Covered: `#[cfg(test)] mod tests` in the same file; the five `#[cfg(test)] pub(crate) mod
  test_utils` modules (all verified `cfg(test)`-gated).
- Not covered: `slatedb/tests/*.rs` and `slatedb/benches/*.rs` are separate crates, so the
  lib's crate-root attribute does not reach them (17 + 7 hits at `max-segments = 1`; 0 at
  `= 2`). Fix per file with `#![allow(clippy::absolute_paths)]` if wanted.
- Blind spot: test helpers *not* behind `cfg(test)` (e.g. feature-gated) would be treated as
  production code. None exist today.

## 3. The numbers

Production code only (`cargo clippy --workspace --lib --bins`; test targets never compiled),
allow-list `["std", "core", "alloc"]` in both columns:

| | `max-segments = 1` | `max-segments = 2` |
|---|---|---|
| Total violations | **963** | **252** |
| Distinct paths | 277 | 123 |
| Our own code (`crate::`, `slatedb*::`) | 557 | 141 |
| Third-party crates | 406 | 111 |
| Catches all 3 reviewed cases | yes | no (misses the 2-segment one) |

Including test targets: 963 → 1033 at `= 1`, 252 → 161 at `= 2`
(the delta comes from crates without the `cfg_attr` exemption plus `tests/` directories).

### Per crate

`max-segments = 1`

| Crate | Violations |
|---|---|
| `slatedb` | 731 |
| `bindings/uniffi` | 169 |
| `examples` | 22 |
| `slatedb-bencher` | 17 |
| `slatedb-cli` | 13 |
| `slatedb-common` | 8 |
| `slatedb-txn-obj` | 3 |

`max-segments = 2`

| Crate | Violations |
|---|---|
| `slatedb` | 174 |
| `bindings/uniffi` | 57 |
| `examples` | 8 |
| `slatedb-common` | 7 |
| `slatedb-bencher` | 5 |
| `slatedb-cli` | 1 |

### Top offenders at `max-segments = 1`

| Count | Path |
|---|---|
| 322 | `crate::Error` |
| 17 | `object_store::Error` |
| 14 | `object_store::Result<()>` |
| 14 | `tokio::spawn` |
| 12 | `anyhow::Result<()>` |
| 11 | `tokio::task::coop::consume_budget` |
| 11 | `tracing::Level` |
| 10 | `object_store::Error::Generic` |
| 10 | `slatedb::Settings` |
| 9 | `chrono::Duration` |
| 8 | `ulid::Ulid` |
| 8 | `serde_json::to_string` |
| 7 | `object_store::Result<PutResult>` |
| 6 | `object_store::Error::NotFound` |
| 6 | `object_store::Result<Path>` |
| 6 | `object_store::Result<ObjectMeta>` |
| 6 | `object_store::path::Path` |
| 6 | `tokio::task::spawn_blocking` |
| 6 | `async_channel::unbounded` |
| 6 | `crate::prefix_extractor::PrefixExtractor` |
| 6 | `tokio::runtime::Handle` |
| 5 | `object_store::Result<GetResult>` |
| 5 | `serde_json::Error` |
| 5 | `futures::stream::iter` |
| 5 | `crate::error::SlateDBError` |

### Top offenders at `max-segments = 2`

| Count | Path |
|---|---|
| 11 | `tokio::task::coop::consume_budget` |
| 6 | `object_store::path::Path` |
| 6 | `tokio::task::spawn_blocking` |
| 6 | `crate::prefix_extractor::PrefixExtractor` |
| 6 | `tokio::runtime::Handle` |
| 5 | `futures::stream::iter` |
| 5 | `crate::error::SlateDBError` |
| 5 | `crate::types::RowEntry` |
| 5 | `tokio::sync::watch::Receiver<DbStatus>` |
| 5 | `slatedb::admin::CloneBuilder` |
| 5 | `tracing::field::Field` |
| 4 | `tokio::time::sleep` |
| 4 | `crate::compaction_worker::COMPACTION_WORKER_TASK_NAME` |
| 4 | `crate::compactor_state::WorkerSpec` |
| 4 | `slatedb::config::WriteOptions` |
| 4 | `slatedb::config::CheckpointOptions` |
| 4 | `slatedb::admin::AdminBuilder<String>` |
| 4 | `slatedb::config::GarbageCollectorDirectoryOptions` |
| 4 | `slatedb::config::GarbageCollectorScheduleOptions` |
| 3 | `tokio::time::Instant` |
| 3 | `crate::error::Error` |
| 3 | `slatedb_common::metrics::MetricsRecorderHelper` |
| 3 | `crate::utils::WatchableOnceCellReader<Result<(), SlateDBError>>` |
| 3 | `tokio::sync::oneshot::error::RecvError` |
| 3 | `crate::mem_table::ImmutableMemtable` |

### What only `max-segments = 1` catches (2-segment paths)

| Count | Path | Kind |
|---|---|---|
| 322 | `crate::Error` | own |
| 17 | `object_store::Error` | third-party |
| 14 | `tokio::spawn` | third-party |
| 14 | `object_store::Result<()>` | third-party |
| 12 | `anyhow::Result<()>` | third-party |
| 11 | `tracing::Level` | third-party |
| 10 | `slatedb::Settings` | own |
| 10 | `object_store::Error::Generic` | third-party |
| 9 | `chrono::Duration` | third-party |
| 8 | `ulid::Ulid` | third-party |
| 8 | `serde_json::to_string` | third-party |
| 7 | `object_store::Result<PutResult>` | third-party |
| 6 | `object_store::Result<Path>` | third-party |
| 6 | `object_store::Result<ObjectMeta>` | third-party |
| 6 | `object_store::Error::NotFound` | third-party |
| 6 | `async_channel::unbounded` | third-party |
| 5 | `serde_json::Error` | third-party |
| 5 | `object_store::Result<GetResult>` | third-party |
| 5 | `foyer::Error` | third-party |
| 5 | `async_channel::Receiver<T>` | third-party |

### Files with the most violations (`max-segments = 1`)

| Count | File |
|---|---|
| 60 | `slatedb/src/admin.rs` |
| 53 | `slatedb/src/db.rs` |
| 40 | `slatedb/src/db_cache/mod.rs` |
| 38 | `slatedb/src/cached_object_store/object_store.rs` |
| 38 | `bindings/uniffi/src/config.rs` |
| 37 | `slatedb/src/retrying_object_store.rs` |
| 32 | `slatedb/src/format/sst.rs` |
| 31 | `slatedb/src/db/builder.rs` |
| 31 | `slatedb/src/ops.rs` |
| 29 | `slatedb/src/db_transaction.rs` |
| 28 | `slatedb/src/cached_object_store/storage_fs.rs` |
| 28 | `slatedb/src/db_reader.rs` |

## 4. What each setting exposes

### `max-segments = 1` — 963 sites

Correct by the letter of the rule: every absolute path must be imported. It is the only
setting that covers all three reviewed cases. Note it does **not** ban the idiomatic
`use std::fmt;` + `fmt::Display` style — relative paths are not absolute paths.

Problems it exposes:

1. **`crate::Error` alone is 322 sites — 33% of everything.** Almost certainly
   `Result<T, crate::Error>` in signatures. Some of these may be deliberate: the workspace
   also has `std::error::Error`, `object_store::Error`, `serde_json::Error`, `foyer::Error`,
   `figment::Error`. **Only the authors know how much of this is disambiguation rather than
   laziness**, and the answer moves the cleanup from 963 to 641.
2. **406 of the 963 sites are third-party paths** — `tokio::spawn` (14),
   `object_store::Result<...>` (~40 across variants), `anyhow::Result<()>` (12),
   `serde_json::to_string` (8), `chrono::Duration` (9), `ulid::Ulid` (8). Inline
   `tokio::spawn(...)` is widespread in the Rust ecosystem; calling it a defect is a real
   style position, not an obvious bug fix.
3. Touches 7 of 8 workspace crates, so the cleanup PR is wide.

### `max-segments = 2` — 252 sites

Problems it exposes:

1. **It does not enforce what the review asked for.** `slatedb_txn_obj::DirtyObject` from
   #2006 passes, so the same comment can be written again on the next PR.
2. **It lets `crate::Error` through** (322 sites) — the single most common inline
   path in the codebase stays unchecked.
3. It is not a principled line: "at most two segments" has no meaning beyond "quieter".

## 5. Prior art: what nine well-known Rust projects actually do

Surveyed by reading the actual files in each repo (root `Cargo.toml` lints tables, `clippy.toml`,
crate-root attributes, CI configs, style docs) on 2026-08-11.

| Project | `clippy::absolute_paths` | Lint mechanism | Test code exempt? | Where an FQN/import rule lives |
|---|---|---|---|---|
| tokio | not enabled | crate-root attrs; `[workspace.lints]` used only for `unexpected_cfgs` | no (`--tests`) | nowhere |
| openai/codex | not enabled | `[workspace.lints.clippy]`, 36 lints, all `deny` | **partly** — `allow-unwrap-in-tests`, `allow-expect-in-tests` | nowhere (`rustfmt imports_granularity = "Item"`) |
| rust-analyzer | not enabled — **`restriction = { level = "allow", priority = -1 }`** blanket-allows the group it belongs to | lint groups with `priority = -1`, then 20 individual overrides | no (`--all-targets`) | `docs/book/src/contributing/style.md` marks `impl std::fmt::Display for X` as **BAD**; `use std::fmt;` + `fmt::Display` is GOOD |
| risingwave | not enabled | all `warn`, CI escalates; plus a `dylint` crate for **one** custom rule | no | nowhere (`rustfmt imports_granularity = "Module"`) |
| deno | not enabled | **no `[workspace.lints]` at all**; 61 per-crate `clippy.toml`, levels via CLI flags | no (`--all-targets`) | nowhere |
| ruff | not enabled | `pedantic` group `warn` + 20 opt-outs | no | `AGENTS.md`: imports at top of file, never local |
| **uv** | not enabled | same as ruff | no | **`AGENTS.md`: "PREFER top-level imports over local imports or fully qualified names"** |
| zed | not enabled | 5 specific `deny` lints; **`style = { level = "allow", priority = -1 }`** with a written rationale | no (`--all-targets`) | nowhere (`.rules` agent file has no such rule) |
| materialize | not enabled | generated `# BEGIN LINT CONFIG` block; **`style` and `complexity` groups allowed**, ~60 lints re-enabled | no (`--all-targets`) | `doc/developer/style.md` §Imports — and it states CI cannot enforce import style |
| influxdb (IOx) | not enabled | small `deny` table + per-crate `#![warn(...)]` headers | no | nowhere |

### Three findings that bear directly on this proposal

**a. Zero of nine enable the lint, and two reject the whole category in writing.**

zed, root `Cargo.toml`:

> ```
> # We currently do not restrict any style rules
> # as it slows down shipping code to Zed.
> #
> # Running ./script/clippy can take several minutes, and so it's
> # common to skip that step and let CI do it. Any unexpected failures
> # (which also take minutes to discover) thus require switching back
> # to an old branch, manual fixing, and re-pushing.
> style = { level = "allow", priority = -1 }
> ```

materialize, `misc/python/materialize/cli/gen-lints.py`:

> "The style and complexity lints frustrated too many engineers and caused more bikeshedding than
> they saved. These lint categories are largely a matter of opinion."

materialize, `doc/developer/style.md` §Imports:

> "There are unfortunately no good ways of enforcing a consistent import style in CI."

Enabling `clippy::absolute_paths` here would be setting a precedent, not following one.

**b. Where the rule does exist, it is prose — and one project words it exactly like ours.**

uv, `AGENTS.md`:

> `- PREFER top-level imports over local imports or fully qualified names`

rust-analyzer, `docs/book/src/contributing/style.md` — note this contradicts allow-listing `std`:

> ```rust
> // GOOD
> use std::fmt;
> impl fmt::Display for RenameError { ... }
>
> // BAD
> impl std::fmt::Display for RenameError { ... }
> ```
> **Rationale:** overall, less typing. Makes it clear that a trait is implemented, rather than used.

SlateDB has neither a `CLAUDE.md` nor an `AGENTS.md`, so today the rule is enforced only by reviewers.

**c. Nobody exempts test code with a crate-root `cfg_attr`.**

Eight of nine lint test code at exactly the same level via `--all-targets` / `--tests`; only codex softens
anything, and it does so with `clippy.toml`'s `allow-unwrap-in-tests` / `allow-expect-in-tests`.
`#![cfg_attr(test, allow(...))]` at a crate root: **0 of 9**. The industry pattern is to relax at the
specific site with a mandatory reason — deno enforces this with
`-D clippy::allow_attributes_without_reason`; ruff and uv require `#[expect()]` over `#[allow()]`
so suppressions expire on their own.

### Adjacent patterns worth stealing regardless of this decision

1. **A check that no crate forgets `[lints] workspace = true`** — three of nine do this:
   codex (`.github/scripts/verify_cargo_workspace_manifests.py`), zed
   (`tooling/xtask/src/tasks/package_conformity.rs`), materialize (`assert` inside `gen-lints.py`).
   SlateDB has this exact gap: `slatedb-common` and `slatedb-txn-obj` have no `[lints]` section, so the
   existing `unreachable_pub = "warn"` silently does not apply to them.
2. **`clippy.toml` `await-holding-invalid-types`** — used by codex for `tokio::sync::MutexGuard` /
   `RwLockReadGuard` / `RwLockWriteGuard`, and by materialize for `tracing::span::Entered` with
   `reason = "use tracing::instrument ... instead"`. This is the mechanized form of the invariant that
   [#2004](https://github.com/slatedb/slatedb/issues/2004) violated — a guard held across a call into
   user code. Far more valuable than a style lint.
3. **`disallowed-methods` / `disallowed-types` with `reason` and `replacement`** — every project in the
   survey that has a `clippy.toml` uses this; zed and influxdb also fill in `replacement`, so the error
   names the fix. SlateDB already does this for DST determinism (`rand::rngs::ThreadRng` →
   `slatedb_common::DbRand`); the pattern generalizes.
4. **One wrapper script as the single source of clippy flags** (zed's `script/clippy`, deno's
   `tools/lint.js`) so local and CI runs cannot drift.

## 6. The adjacent lint that survives the ambiguity objection: `unused_qualifications`

`unused_qualifications` is a **rustc** lint (allow by default), not a clippy one. It fires only when a
qualification is *provably redundant* — the item is already in scope, so removing the prefix resolves
to the same thing. Deliberate disambiguation therefore never triggers it: if you write
`object_store::Error` to distinguish it from an imported `crate::Error`, the un-imported one cannot be
shortened and is not flagged.

Measured with `cargo clippy --workspace --lib --bins -- -W unused_qualifications` (production code only):

| Repo | Violations |
|---|---|
| tokio | **0** |
| rust-analyzer | **0** |
| **slatedb** | **142** |

Neither reference project enables the lint; they are simply clean. SlateDB is the outlier.

The 142 span **56 distinct spellings**, and only 22% involve `Error`:

| Count | Spelling |
|---|---|
| 13 | `Bound::Included` |
| 11 | `Bound::Unbounded` |
| 10 | `crate::Error::from` |
| 9 | `SsTableId::Compacted` |
| 8 | `Bound::Excluded` |
| 6 | `crate::Error` |
| 6 | `std::mem::size_of::<u16>` |
| 5 | `crate::error::SlateDBError` |
| 4 | `rand_xorshift::XorShiftRng::from_os_rng` |
| 3 | `atomic::Ordering::SeqCst`, `crate::mem_table::ImmutableMemtable`, `std::any::Any`, … |

Three distinct causes, all mechanical:

```rust
// 1. item imported, full path written anyway — slatedb/src/compactor.rs
use crate::error::{Error, SlateDBError};                  // line 82
fn validate(...) -> Result<(), Error>                     // line 149  short name
pub async fn run(&self) -> Result<(), crate::Error>       // line 376  full path, same type
return Err(crate::Error::from(SlateDBError::InvalidCompaction));  // line 191  mixed in one line

// 2. enum variants imported directly — slatedb/src/bytes_range.rs (32 of the 142)
use std::ops::Bound::{Excluded, Included, Unbounded};
Bound::Included(k) => Bound::Included(k.as_ref())         // `Included(k)` suffices

// 3. already in the prelude
std::mem::size_of::<u16>()                                // `size_of::<u16>()` suffices
```

Repo-wide, `crate::Error` appears **384** times while only **3 files** import `Error` — so the vast
majority of `crate::Error` uses are *not* redundant (nothing to shorten) and stay untouched by this
lint. The 142 are exactly the inconsistent subset.

### Which convention do those projects actually follow?

Same static yardstick across repos (no compilation; counts `use` forms and use-site path shapes):

| Repo | `use …::Type;` | `use …::module;` | use-site paths with 2+ segments | use-site `module::Type` |
|---|---|---|---|---|
| slatedb | 4,736 | 276 | 1,129 | 3,634 |
| tokio | 3,171 | 569 | 522 | 3,147 |
| rust-analyzer | 11,428 | 707 | 3,168 | 18,109 |
| ruff | 20,416 | 1,643 | 2,484 | 14,529 |
| risingwave | 40,102 | 2,500 | 3,642 | 7,257 |

The industry norm is **not** "never write a qualified path". Two styles coexist everywhere:

1. import the item, use the short name (`use crate::error::Error;` → `Error`);
2. import the module, keep one segment (`use std::fmt;` → `fmt::Display`) — the largest column in
   every repo, and the style rust-analyzer's `style.md` *mandates* for `hir::` / `ast::` items.

Deep inline paths are a minority everywhere but never zero (tokio 522, rust-analyzer 3,168). What is
zero is the **mixture**: importing a symbol and then still spelling out its full path in the same
file. That, and only that, is the line the reference projects hold.

## 7. Decisions needed

1. **Is inline `crate::Error` (and 2-segment paths generally) something we want to ban?**
   This is the 33%-of-the-work question, and it decides `max-segments` 1 vs 2.
2. **Do third-party inline paths count?** `tokio::spawn`, `object_store::Result<()>`,
   `anyhow::Result<()>`. If not, the only expressible alternative is a longer
   `absolute-paths-allowed-crates` list, and any such list is arbitrary (per-symbol
   granularity does not exist).
3. **Scope:** all workspace crates, or `slatedb` first? Note that pulling in
   `slatedb-common` and `slatedb-txn-obj` requires giving them `[lints] workspace = true`,
   which they currently lack — that also subjects them to `unreachable_pub` for the first
   time and is better done as a separate change.

### Recommendation

The survey in §5 changed my recommendation. Ordered by value per unit of disruption:

1. **Write the rule down where humans and coding agents will read it.** SlateDB has no
   `CLAUDE.md` and no `AGENTS.md`. uv's one-liner is the exact rule we want and is proven in a
   comparable project: `- PREFER top-level imports over local imports or fully qualified names`.
   Zero cleanup, zero CI change, and it targets the actual source of the recurring review comment.
   rust-analyzer's `style.md` is the model for adding a *Rationale:* to each rule.
2. **Close the `[lints] workspace = true` gap** in `slatedb-common` and `slatedb-txn-obj`, and add a
   check so a new crate cannot silently opt out (three of nine surveyed projects have exactly this
   check). Independent of the path question; it is a latent-bug fix, not a style change.
3. **Add `await-holding-invalid-types` to `.config/clippy.toml`** for the guard types involved in
   [#2004](https://github.com/slatedb/slatedb/issues/2004). codex and materialize both use this key.
   This mechanizes the invariant that actually caused a bug ("setting a metric is a contained atomic
   operation") rather than a formatting preference.
4. **Only then, if maintainers want the path rule mechanized:** `max-segments = 1` with
   `absolute-paths-allowed-crates = ["std", "core", "alloc"]` — the only setting that covers all
   three reviewed cases — at a cost of 963 sites with zero auto-fixes. Note two caveats: it would be
   the first project in the survey to enable this lint, and rust-analyzer's style guide argues
   against the `std` exemption (it calls `impl std::fmt::Display` BAD and wants
   `use std::fmt;` + `fmt::Display`), so an empty allow-list is the more internally consistent
   choice at a still higher cost.

`max-segments = 2` is not recommended in any form: it does not prevent the reviewed comments from
recurring, so it buys a 252-site cleanup without buying the rule.

## 8. Reproducing

```bash
# config used for the numbers above (.config/clippy.toml, symlinked from each crate)
absolute-paths-max-segments = 1   # or 2
absolute-paths-allowed-crates = ["std", "core", "alloc"]

# switch
# root Cargo.toml: [workspace.lints.clippy] / absolute_paths = "warn"

# production code only
cargo clippy --workspace --lib --bins --message-format=json \
  | jq -r 'select(.message.code.code=="clippy::absolute_paths")' | wc -l

# what CI runs
RUSTFLAGS="-Dwarnings" cargo hack clippy --each-feature --no-dev-deps --workspace
RUSTFLAGS="-Dwarnings" cargo hack clippy --workspace --tests
```
