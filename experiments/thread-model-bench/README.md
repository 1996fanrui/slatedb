# SlateDB write path: a per-write cross-thread round-trip, and an 8x fix

While validating SlateDB as a Flink state backend we found writes bottlenecked by the per-write
hand-off to the batch-writer task. This documents the root cause and a configuration-level fix worth
**8x**, both backed by benchmarks in this directory.

**Conclusion:** registering only the batch-writer on a current-thread runtime that the caller also
writes through (`DbBuilder::with_write_runtime`, +13 lines), and building that runtime with
`enable_time()` rather than `enable_all()` so its park does not cost a `kevent` syscall, takes a
single-threaded put from **9.62 µs to 1.20 µs — 8x** with the write path and its ordering guarantees
unchanged.

## The problem

The hand-off pays for itself when many threads write concurrently and hard enough to collide. With a
single writer, or a few low-rate writers, there is nothing to batch and no contention to amortize,
so every write pays the full round-trip for no benefit. Flink is the extreme case: keyed state is
written from one thread, so the cost is paid on 100% of writes.

**`thread-model-bench`** isolates the hand-off with a trivial `+1` payload, single-threaded — the
work inline, then handed to another task on the *same* OS thread, then to a task on a *different* OS
thread (SlateDB's shape):

| same thread, one task | same thread, two tasks | two threads, two tasks |
|---|---|---|
| 1.95 ns · 514 M ops/s | 120 ns · 8.3 M ops/s | 9.79 µs · 102 k ops/s |

The hand-off itself is ~80x cheaper when both tasks share an OS thread: nothing has to be woken.

## The fix

The round-trip is cross-thread only because `block_on` drives the caller's future on the calling
thread while `spawn` puts the writer task on a worker. `DbBuilder::with_write_runtime(Handle)` (this
branch, +13 lines) registers only the batch-writer on a given runtime; flusher, compactor, GC and the
task monitor stay where they were. Point it at a current-thread runtime the caller also writes
through, and the writer task runs on the caller's thread while background work keeps its workers.

**`enable_all()` makes this slower than doing nothing** — 14.5 µs vs 9.6 µs. That runtime's single
thread also drives the IO/timer driver, and `block_on`'s loop parks unconditionally each iteration
even with a zero timeout, so every write pays a `kevent` syscall; a CPU profile put **46% of the
caller thread's wall time inside that one syscall**. It is not background components competing for
the thread — moving the batch-writer alone with `enable_all()` still on does not move the number
(`slatedb_native_split_io`).

Build the caller's runtime with `enable_time()` instead: timer yes, IO driver no. Dropping the timer
as well is tempting and unsafe. Two call sites sleep on this runtime — `MonotonicClock::now()` on
clock skew (`clock.rs`) and `monitor_first_write` on the first durable write (`batch_write.rs`) —
and neither is reached on the normal path, so a driverless runtime benchmarks fine and would fail
only once the clock steps backwards or a caller asks for durability.

The write path is untouched, so sequence assignment and the memtable write still happen inside the
one batch-writer task, one message at a time. The cost is deployment shape: one client thread per
`Db` per current-thread runtime, since several client threads sharing one would serialize on its
scheduler core. That fits Flink, where writers are tied to partitions.

## Benchmark

**`write-bench`**: single-threaded `Db::put_with_options`, 100 keys in steady state, WAL disabled, no
per-write durability, 60 MiB memtable, local filesystem. Same write path in every row; the only
variable is which runtime the batch-writer lives on. Each row opens a fresh DB and closes it before
the next.

| Mode | per put | throughput | vs today |
|---|---|---|---|
| `rocksdb` (synchronous inline, reference) | 0.43 µs | 2,353 k ops/s | 22.6x |
| **`slatedb_native_split_runtime`** (`enable_time`) | **1.20 µs** | **835 k ops/s** | **8.0x** |
| `slatedb_native` (today) | 9.62 µs | 104 k ops/s | 1x |
| `slatedb_native_split_io` (split, `enable_all`) | 14.45 µs | 69 k ops/s | 0.67x |
| `slatedb_native_current_thread` (whole DB, `enable_all`) | 14.54 µs | 69 k ops/s | 0.66x |

Apple M5 Max, arm64, one run. RocksDB is here only to place the numbers on a familiar scale; where
the remaining 2.8x goes was not investigated.

> Not measured: multiple concurrent client threads, and configurations with the WAL enabled or
> per-write durability. Flink needs neither.

> Early writeup to start the discussion; if the direction holds up, the design belongs in an RFC.
