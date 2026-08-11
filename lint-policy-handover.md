# Handover: mechanizing SlateDB's "import it, don't spell out the path" rule

**Status.** Investigation finished 2026-08-11. Nothing proposed upstream yet. This branch
(`clippy-absolute-paths`, forked from `community/main` @ `d12891f1`) exists as a backup of the
investigation: two documents plus a working, verified lint configuration that is **not** meant to be
pushed upstream as-is.

Companion document: [`absolute-paths-lint-analysis.md`](./absolute-paths-lint-analysis.md) — all raw
data, per-crate and per-path tables, the nine-project survey, and reproduction commands.

---

## 1. The decision

Two rules, in this order. The first is the deliverable; the second is a question we ask while
delivering the first.

| | Rule | Tool | Sites to fix | Merge odds |
|---|---|---|---|---|
| **PR 1 — do this** | If a symbol is already in scope, use the short name | `unused_qualifications` (rustc, built-in) | **142** | high |
| **PR 2 — ask first** | At a use site, keep at most one qualifying segment | `clippy::absolute_paths` with `absolute-paths-max-segments = 2` | **252** | plausible — it is the mainstream style |

Rationale for the split: PR 1's rule has **no exception clause** and no judgement calls — the compiler
proves each violation is redundant. PR 2's rule is a real style position, so it should be the
maintainers' call, not ours. Raise it in a comment on PR 1; if they want it, land it in the same PR or
a follow-up (there are enough sites that a separate PR is probably cleaner).

### Why these two and not the alternatives

- `absolute-paths-max-segments = 1` — **rejected.** 963 sites, and 459 of them (48%) are `Error` /
  `Result` names from seven different crates. This codebase has six distinct `Error` types and two
  `Result` aliases; you cannot import two `Result`s into one file, so the prescribed "fix" is
  `use anyhow::Result as AnyhowResult`, which reads worse than the inline path. The rule is simply
  wrong for those sites.
- A dylint crate — **rejected.** RisingWave pays for a separate crate, its own pinned
  `rust-toolchain`, its own `Cargo.lock`, and a dedicated CI step to enforce **one** rule. Not worth it
  for path style.
- A diff-scoped CI check (only new/changed lines) — **rejected.** It permanently splits old and new
  code and does not settle what the rule actually is.

---

## 2. What was measured (all numbers verified locally, clippy 1.91.0)

### 2.1 `unused_qualifications` — the outlier finding

```bash
cargo clippy --workspace --lib --bins -- -W unused_qualifications      # production code only
```

| Repo | Violations |
|---|---|
| tokio (`-p tokio --lib --all-features`) | **0** |
| rust-analyzer (`--workspace --lib`) | **0** |
| **slatedb** | **142** |

Neither reference project *enables* this lint — they are clean anyway. That is the argument to bring
upstream: the baseline for a well-maintained Rust project is zero, and we are at 142.

Per crate: `slatedb` 134, `slatedb-bencher` 4, `examples` 2, `slatedb-txn-obj` 1, `bindings/uniffi` 1.

Densest files: `bytes_range.rs` 32, `compactor.rs` 19, `flatbuffer_types.rs` 12, `types.rs` 10,
`db_state.rs` 5, `memtable_flusher/mod.rs` 5, `db_status.rs` 4, `format/row.rs` 4, `utils.rs` 4.

56 distinct spellings; only 22% involve `Error`. Top: `Bound::Included` 13, `Bound::Unbounded` 11,
`crate::Error::from` 10, `SsTableId::Compacted` 9, `Bound::Excluded` 8, `crate::Error` 6,
`std::mem::size_of::<u16>` 6, `crate::error::SlateDBError` 5.

Three mechanical causes:

```rust
// (a) item imported, full path written anyway — slatedb/src/compactor.rs
use crate::error::{Error, SlateDBError};                          // line 82
fn validate(...) -> Result<(), Error>                             // line 149  short name
pub async fn run(&self) -> Result<(), crate::Error>               // line 376  full path, same type
return Err(crate::Error::from(SlateDBError::InvalidCompaction));  // line 191  both in one line

// (b) enum variants imported directly — slatedb/src/bytes_range.rs (32 of the 142)
use std::ops::Bound::{Excluded, Included, Unbounded};
Bound::Included(k) => Bound::Included(k.as_ref())                 // `Included(k)` suffices

// (c) already in the prelude
std::mem::size_of::<u16>()                                        // `size_of::<u16>()` suffices
```

No false-positive risk by construction: the lint fires only when the compiler can prove the prefix is
removable. Deliberate disambiguation (e.g. writing `object_store::Error` in a file that imports
`crate::Error`) is never flagged, because the un-imported name cannot be shortened.

Scale check: `crate::Error` appears **384** times repo-wide, but only **3 files** import `Error`. So
the overwhelming majority of `crate::Error` uses are not redundant and this lint leaves them alone.

### 2.2 `clippy::absolute_paths` — the optional rule

`absolute-paths-max-segments = 2` means "an absolute path may have at most two segments", i.e. **at
most one qualifying segment at the use site**. Relative paths are not absolute paths, so the idiomatic
`use std::fmt;` + `fmt::Display` form is never touched.

| Use-site form | Segments | Allowed at `= 2` |
|---|---|---|
| `Error` | 0 | yes |
| `fmt::Display` (after `use std::fmt;`) | — | yes, not even examined |
| `crate::Error`, `object_store::Error`, `tokio::spawn` | 2 | yes |
| `crate::error::SlateDBError`, `object_store::path::Path` | 3 | **no** |
| `tokio::task::coop::consume_budget` | 4 | **no** |

Every flagged site has a fix in one of the two mainstream styles, and the lint does not dictate which:

```rust
// violation
fn f(p: object_store::path::Path)

// fix A — import the item
use object_store::path::Path;
fn f(p: Path)

// fix B — import the module (use this when the file also has std::path::Path)
use object_store::path;
fn f(p: path::Path)
```

Counts, production code only, `cargo clippy --workspace --lib --bins`:

| Config | Violations |
|---|---|
| `= 2`, allow-list `["std", "core", "alloc"]` | **252** (our own code 141, third-party 111) |
| `= 2`, also allow-listing the 11 third-party crates actually used inline | 141 |
| `= 1`, allow-list `["std", "core", "alloc"]` | 963 — rejected, see §1 |

Per crate at `= 252`: `slatedb` 174, `bindings/uniffi` 57, `examples` 8, `slatedb-common` 7,
`slatedb-bencher` 5, `slatedb-cli` 1.

**Prefer the 252 variant** (standard library only). Its justification is one sentence — "standard
library paths may stay inline by community convention; everything else keeps at most one qualifying
segment" — whereas any longer allow-list requires explaining why `tokio` is exempt and `serde_json` is
not.

### 2.3 Coverage of the two review comments that started this

| PR | Flagged code | Segments | `unused_qualifications` | `absolute_paths = 2` |
|---|---|---|---|---|
| [#1939](https://github.com/slatedb/slatedb/pull/1939) | `crate::mem_table::KVTableMetadata` | 3 | no (not imported at the time) | **yes** |
| [#2006](https://github.com/slatedb/slatedb/pull/2006#discussion_r3754004177) | `crate::manifest::Manifest` | 3 | no | **yes** |
| #2006 | `slatedb_txn_obj::DirtyObject` | 2 | no | no — legal by design |

The third one stays legal under our rule: one qualifying segment is exactly the shape of
`fmt::Display`. Say this explicitly upstream — the proposal covers the part of the feedback that
matches industry convention, and does not promise to cover reviewer preference beyond it.

### 2.4 Mechanism facts you will need (all verified)

- **Config location.** `clippy.toml` at the workspace root is **ignored**; clippy resolves config from
  each crate's manifest directory. This repo already solves that: `.config/clippy.toml` is the single
  source and each crate has a symlink `clippy.toml -> ../.config/clippy.toml`. Crates that currently
  have the symlink: `slatedb`, `slatedb-common`, `slatedb-txn-obj`. Missing (need one if the lint is
  enabled workspace-wide): `bindings/uniffi`, `slatedb-cli`, `slatedb-bencher`, `slatedb-dst`,
  `examples`.
  Do not `cp` onto those symlinks — it writes *through* them and destroys `.config/clippy.toml`
  (this happened during the investigation; recovered with `git checkout`).
- **`clippy.toml` cannot enable a lint**, only configure it. Enabling happens in
  `[workspace.lints.*]` (root `Cargo.toml`) or a crate-root `#![warn(...)]`.
- **`absolute-paths-allowed-crates` is crate-level only.** Adding `"object_store::Result"` or
  `"tokio::task"` changes nothing (963 → 963) and clippy silently ignores the unusable entries. So
  "flag only our own crates" cannot be expressed without allow-listing all 445 external packages.
- **No auto-fix.** Zero of the diagnostics carry a machine-applicable suggestion, for either lint.
  `cargo clippy --fix` does nothing. All cleanup is manual.
- **CI needs no workflow change.** `.github/workflows/pr.yaml` already sets `RUSTFLAGS: "-Dwarnings"`,
  so a `"warn"` level in `[workspace.lints]` fails CI. It runs
  `cargo hack clippy --each-feature --no-dev-deps --workspace` (production, `cfg(test)` off) and
  `cargo hack clippy --workspace --tests` (test targets, `cfg(test)` on).
  Note `--each-feature` compiles feature combinations this investigation did not, so the real counts
  may be slightly higher than the numbers above.
- **`[lints] workspace = true` gap.** `slatedb-common` and `slatedb-txn-obj` do not declare it, so the
  existing `unreachable_pub = "warn"` silently does not apply to them. Verified: injecting a `pub` item
  in a private module produced 0 warnings before adding `[lints]` and a warning after. Adding it today
  surfaces **zero** existing violations, so this is latent-risk cleanup, not a bug fix with symptoms.
  Three of the nine surveyed projects have a CI check for exactly this (codex
  `.github/scripts/verify_cargo_workspace_manifests.py`, zed
  `tooling/xtask/src/tasks/package_conformity.rs`, materialize `assert` in `gen-lints.py`).

### 2.5 Test code

`slatedb/src/lib.rs` already uses `#![cfg_attr(test, allow(...))]` for five lints (`unwrap_used`,
`panic`, `disallowed_types`, `disallowed_methods`, `disallowed_macros`), so that idiom is house style.
Note, though, that **0 of the 9 surveyed projects use it** — they lint test code at the same level via
`--all-targets` / `--tests` and relax at the specific site with a mandatory reason (deno enforces
`-D clippy::allow_attributes_without_reason`; ruff and uv require `#[expect()]` over `#[allow()]` so
suppressions expire).

Decision for our PRs: **do not exempt test code.** The 142 `unused_qualifications` sites are
production-only counts; measure the test-target number before starting and clean those too, so the
rule needs no `cfg_attr`. If the volume turns out unreasonable, fall back to the house `cfg_attr`
idiom and say so in the PR.

### 2.6 What the nine surveyed projects do (summary; details in the companion doc)

- **0 of 9 enable `clippy::absolute_paths`.** rust-analyzer blanket-allows the whole `restriction`
  group it belongs to; zed allows the whole `style` group with a written rationale about velocity;
  materialize allows `style` and `complexity` because they "frustrated too many engineers and caused
  more bikeshedding than they saved", and its style guide states outright that CI cannot enforce
  import style.
- Where the rule exists, it is prose: uv's `AGENTS.md` says
  `- PREFER top-level imports over local imports or fully qualified names`; rust-analyzer's
  `style.md` marks `impl std::fmt::Display for X` as BAD and wants `use std::fmt;` + `fmt::Display`,
  with a `Rationale:` line per rule.
- Deep inline paths are a minority **but present everywhere** (tokio 522, rust-analyzer 3168,
  risingwave 3642, ruff 2484 by static scan). Normalized per 1000 lines, SlateDB is 8.0 against a
  3.9–6.7 industry range — the same order of magnitude. **Do not argue upstream that we overuse deep
  paths; we do not.** The only defensible outlier is 142 vs 0.
- The line every reference project holds is narrower than "avoid qualified paths": never import a
  symbol *and* spell out its full path in the same file.

---

## 3. PR plan

### PR 1 — enable `unused_qualifications` (the deliverable)

Title: `Use short names for items already in scope`

Contents:
1. Clean up the 142 production sites (plus whatever the test targets add).
2. Add to the root `Cargo.toml`:
   ```toml
   [workspace.lints.rust]
   unused_qualifications = "warn"
   ```
3. Add `[lints] workspace = true` to `slatedb-common/Cargo.toml` and `slatedb-txn-obj/Cargo.toml` —
   without it the new rule does not apply to them. Mention in the PR body that this also subjects
   those two crates to the existing `unreachable_pub` for the first time (verified: no new violations).

Commit structure: one commit per crate, and split `slatedb` by directory so each commit is scannable.
Do **not** submit a single 142-file commit.

Body should contain: the 142 / tokio 0 / rust-analyzer 0 comparison, the `compactor.rs` example showing
`SlateDBError` short and `crate::Error` qualified *on the same line*, and the note that the lint has no
exception clause.

### The comment to leave on PR 1 (this is how PR 2 gets proposed)

> While cleaning this up I also measured `clippy::absolute_paths` with
> `absolute-paths-max-segments = 2` — i.e. "at a use site, keep at most one qualifying segment", which
> permits both mainstream styles (`use crate::error::Error;` → `Error`, and `use std::fmt;` →
> `fmt::Display`) and rejects only `crate::error::SlateDBError`-style two-level paths. That is 252
> sites with `absolute-paths-allowed-crates = ["std", "core", "alloc"]`, and it is the rule that would
> actually have caught the two review comments that prompted this. Worth doing as a follow-up, or
> would you rather leave path depth to review?

Do not attach the 252-site diff to PR 1. Wait for an answer.

### PR 2 — conditional on that answer

Title: `Keep at most one qualifying segment at use sites`

Contents:
1. Clean up the 252 sites.
2. `.config/clippy.toml`:
   ```toml
   absolute-paths-max-segments = 2
   absolute-paths-allowed-crates = ["std", "core", "alloc"]
   ```
3. Root `Cargo.toml`: `[workspace.lints.clippy] absolute_paths = "warn"`.
4. Add the missing `clippy.toml` symlinks (`bindings/uniffi`, `slatedb-cli`, `slatedb-bencher`,
   `slatedb-dst`, `examples`) — without them those crates get clippy's default config and the
   standard-library allow-list will not apply.

### Explicitly out of scope

`absolute-paths-max-segments = 1`; a generated 445-entry allow-list; a dylint crate; a diff-scoped CI
script; any attempt to mechanize "do not hold a lock while calling user code" (clippy cannot express
it — `await-holding-invalid-types` only covers guards held across `.await`, which is a different bug
class from [#2004](https://github.com/slatedb/slatedb/issues/2004)).

---

## 4. Work packages for parallel agents

Shared rules for every cleanup agent:

- **Imports only.** No behaviour changes, no renames, no reordering beyond what `cargo fmt` does.
- When shortening would collide with an existing name in the file, prefer importing the *module* and
  keeping one segment (`use object_store::path;` → `path::Path`) over aliasing with `as`.
- After each file group: `cargo fmt --all`, then
  `RUSTFLAGS="-Dwarnings" cargo clippy -p <crate> --all-targets -- -W unused_qualifications`, then the
  crate's tests.
- Do not touch `.config/clippy.toml` with `cp` (symlink hazard, see §2.4).
- One commit per crate or per directory; keep diffs scannable.

| # | Package | Scope | Depends on |
|---|---|---|---|
| A | Clean `unused_qualifications` in `slatedb/src` — 134 sites | `bytes_range.rs` 32, `compactor.rs` 19, `flatbuffer_types.rs` 12, `types.rs` 10, `db_state.rs` 5, `memtable_flusher/mod.rs` 5, `db_status.rs` 4, `format/row.rs` 4, `utils.rs` 4, `batch.rs` 3, remainder spread thin | — |
| B | Clean the other crates — 8 sites | `slatedb-bencher` 4, `examples` 2, `slatedb-txn-obj` 1, `bindings/uniffi` 1 | — |
| C | Measure and clean the **test-target** sites | `cargo clippy --workspace --all-targets -- -W unused_qualifications` minus the prod set | — |
| D | Wire the lint + write the PR body | `[workspace.lints.rust] unused_qualifications = "warn"`, `[lints] workspace = true` in the two crates, PR text with the 142/0/0 table | A, B, C |
| E | *Conditional:* clean `absolute_paths = 2` — 252 sites | `slatedb` 174, `bindings/uniffi` 57, `examples` 8, `slatedb-common` 7, `slatedb-bencher` 5, `slatedb-cli` 1 | maintainer approval on PR 1 |
| F | *Conditional:* wire `absolute_paths` | `.config/clippy.toml` keys, workspace lint entry, 5 missing symlinks | E |

A, B and C are independent and can run in parallel. E is large enough to split per crate.

---

## 5. What is on this branch (do not push upstream as-is)

Commit 1 — the two documents.

Commit 2 — the verified experiment, kept for reference:

| File | Change |
|---|---|
| `.config/clippy.toml` | `absolute-paths-max-segments = 2`, `absolute-paths-allowed-crates = ["std", "core", "alloc"]` |
| `Cargo.toml` | `[workspace.lints.clippy] absolute_paths = "warn"` |
| `slatedb-common/Cargo.toml`, `slatedb-txn-obj/Cargo.toml` | `[lints] workspace = true` |
| `slatedb/src/lib.rs`, `bindings/uniffi/src/lib.rs` | `#![cfg_attr(test, allow(clippy::absolute_paths))]` — see §2.5, the PRs should probably drop this |
| `bindings/uniffi`, `slatedb-cli`, `slatedb-bencher`, `slatedb-dst`, `examples` | new `clippy.toml` symlinks |

The 252 sites are **not** cleaned here, so `RUSTFLAGS="-Dwarnings" cargo clippy` fails on this branch
by design.

### Reproduction

```bash
# unused_qualifications, production code only
cargo clippy --workspace --lib --bins --message-format=json -- -W unused_qualifications \
  | jq -r 'select(.message.code.code=="unused_qualifications") | .message.spans[] | select(.is_primary) | "\(.file_name):\(.line_start)"'

# absolute_paths, with the config already on this branch
cargo clippy --workspace --lib --bins --message-format=json \
  | jq -r 'select(.message.code.code=="clippy::absolute_paths")' | wc -l

# variant configs without touching the repo
CLIPPY_CONF_DIR=/path/to/dir/holding/clippy.toml cargo clippy --workspace --lib --bins -- -W clippy::absolute_paths
```

Note: clippy caches diagnostics. `touch slatedb/src/lib.rs bindings/uniffi/src/lib.rs` before
re-measuring, or the previous run's numbers come back unchanged.
