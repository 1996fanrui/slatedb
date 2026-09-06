# thread-scaling-bench

Measures how `DbBuilder::with_write_runtime` behaves as writer threads are added.

Every write is handed to a single resident batch-writer task and the caller awaits a reply. By
default that task runs on a worker of the runtime the caller drives, so caller and writer sit on
different OS threads and each write pays a cross-thread wake-up. `with_write_runtime` puts the
batch-writer on a current-thread runtime the writing threads drive themselves, turning the wake-up
into a task switch.

## What it runs

Twelve measurements — 1..=6 OS writer threads sharing one `Db`, under two runtime models:

- `native` — today's shape: threads drive a shared multi-thread runtime, batch-writer on a worker.
- `split` — `with_write_runtime` pointed at a current-thread runtime (`enable_time`, no IO driver)
  that the threads drive themselves.

One `Db` across all threads is the case in question: one database has one batch-writer, so one
runtime hosts it however many threads write. Writers are OS threads, not tasks, because
co-location is a property of the thread that calls `block_on`. Each thread overwrites its own 100
keys in a tight loop; 8-byte values, WAL disabled, 60 MiB memtable, local filesystem.

`iter_custom` reports wall time over the iteration count, so the number is the aggregate cost per
write at that thread count, and throughput is its reciprocal.

## Run

```
cargo bench
```

About three minutes (12 groups, each opens and closes its own `Db`).

## Results

Apple M5 Max, arm64, single run.

| threads | native | with_write_runtime | change |
|---|---|---|---|
| 1 | 95 k ops/s | **778 k ops/s** | **8.1x** |
| 2 | 203 k ops/s | **491 k ops/s** | **2.4x** |
| 3 | 367 k ops/s | 376 k ops/s | +2.4% |
| 4 | 341 k ops/s | 361 k ops/s | +6.0% |
| 5 | 367 k ops/s | 384 k ops/s | +4.8% |
| 6 | 362 k ops/s | 379 k ops/s | +4.9% |

No regression at any thread count; the gain is concentrated at low write concurrency. The two
converge from opposite directions — `native` improves as concurrency amortizes the wake-up, `split`
degrades as writers share one scheduler — and meet at ~363k ops/s, which is what a single ordered
batch-writer sustains regardless of how callers reach it.

Not measured: beyond 6 threads, other machines, and configurations with the WAL enabled.
