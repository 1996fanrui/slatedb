//! How writer-thread count interacts with where the batch-writer task lives.
//!
//! Twelve measurements: 1..=6 writer threads, each under two runtime models.
//!
//!   native — today's shape. Every writer thread calls `block_on` on a shared multi-thread
//!            runtime, so the batch-writer task sits on one of its worker threads and each
//!            write costs a cross-thread wake-up.
//!   split  — `DbBuilder::with_write_runtime`. The batch-writer sits on a current-thread
//!            runtime (timer enabled, no IO driver) that the writer threads drive themselves,
//!            so a write costs a task switch. With more than one writer thread they share that
//!            runtime's scheduler.
//!
//! Both models share one `Db` across all writer threads, which is the case under question: one
//! database has one batch-writer, so one runtime hosts it no matter how many threads write.
//!
//! Writers are OS threads, not tasks: co-location is a property of the thread that calls
//! `block_on`, so spawning tasks onto a runtime would not measure anything.
//!
//! Reading the numbers: `iter_custom` reports wall time for all iterations divided by the
//! iteration count, so the reported time is the **aggregate cost per write** at that thread
//! count, and throughput is its reciprocal. Perfect scaling halves it when threads double;
//! flat means the writers are serialized somewhere.
//!
//! Workload per thread: 100 keys of its own overwritten in a tight loop, 8-byte values, WAL
//! disabled, 60 MiB memtable, local filesystem. Each of the twelve opens a fresh database and
//! closes it before the next starts, since background tasks left running would skew what follows.
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::{Builder, Handle, Runtime};

const THREADS: &[usize] = &[1, 2, 3, 4, 5, 6];
const KEYS: u64 = 100;
const MEMTABLE: usize = 60 * 1024 * 1024; // 60 MiB

fn slatedb_settings() -> slatedb::config::Settings {
    let mut s = slatedb::config::Settings::default();
    s.wal_enabled = false;
    s.l0_sst_size_bytes = MEMTABLE;
    s.max_unflushed_bytes = 2 * MEMTABLE;
    s
}

/// Fresh SlateDB at `dir`. When `write_runtime` is set, only the batch-writer moves there.
async fn fresh_slatedb(dir: &std::path::Path, write_runtime: Option<Handle>) -> slatedb::Db {
    use slatedb::object_store::{local::LocalFileSystem, ObjectStore};
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let object_store: Arc<dyn ObjectStore> = Arc::new(LocalFileSystem::new_with_prefix(dir).unwrap());
    let mut builder = slatedb::Db::builder("bench", object_store).with_settings(slatedb_settings());
    if let Some(handle) = write_runtime {
        builder = builder.with_write_runtime(handle);
    }
    builder.build().await.unwrap()
}

/// Spreads `iters` writes over `threads` OS threads, each driving `rt`, and returns the wall
/// time for all of them. Thread `t` writes only into its own 100-key window, so threads do not
/// contend on keys and the memtable grows predictably with the thread count.
fn drive(iters: u64, threads: usize, rt: &Runtime, db: &slatedb::Db) -> Duration {
    let base = iters / threads as u64;
    let remainder = iters % threads as u64;
    let start = Instant::now();
    std::thread::scope(|scope| {
        for t in 0..threads {
            let ops = base + if (t as u64) < remainder { 1 } else { 0 };
            scope.spawn(move || {
                let window = t as u64 * KEYS;
                for i in 0..ops {
                    let key = (window + i % KEYS).to_be_bytes();
                    rt.block_on(db.put(&key, &i.to_be_bytes())).unwrap();
                    black_box(());
                }
            });
        }
    });
    start.elapsed()
}

fn scaling(c: &mut Criterion) {
    let base = std::env::temp_dir().join("thread-scaling-bench");
    let mut group = c.benchmark_group("write_threads");

    for &threads in THREADS {
        // --- today: batch-writer on a worker of the shared multi-thread runtime ---
        {
            let rt = Runtime::new().unwrap();
            let db = rt.block_on(fresh_slatedb(&base.join(format!("native-{threads}")), None));
            group.bench_with_input(BenchmarkId::new("native", threads), &threads, |b, &n| {
                b.iter_custom(|iters| drive(iters, n, &rt, &db))
            });
            rt.block_on(db.close()).unwrap();
        }

        // --- with_write_runtime: batch-writer on the current-thread runtime the writers drive ---
        {
            let background = Runtime::new().unwrap();
            let writer = Builder::new_current_thread().enable_time().build().unwrap();
            let db = background.block_on(fresh_slatedb(
                &base.join(format!("split-{threads}")),
                Some(writer.handle().clone()),
            ));
            group.bench_with_input(BenchmarkId::new("split", threads), &threads, |b, &n| {
                b.iter_custom(|iters| drive(iters, n, &writer, &db))
            });
            // Close goes through the batch-writer, so it needs the runtime hosting it.
            writer.block_on(db.close()).unwrap();
        }
    }

    group.finish();
}

criterion_group!(benches, scaling);
criterion_main!(benches);
