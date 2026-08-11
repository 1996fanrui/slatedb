//! Single-threaded write benchmark: where SlateDB's per-write cross-thread round-trip goes, and what
//! removing it is worth. All functions run the same `Db::put_with_options` under one aligned config,
//! so the only variable is which runtime the batch-writer task lives on.
//!
//! Workload (all functions): 100 keys overwritten in a tight loop, single writer thread, WAL
//! disabled, no per-write durability, ~60 MiB memtable, local filesystem.
//!
//! Isolation: each function opens its OWN fresh database and closes it before the next starts. A
//! SlateDB `Db` runs background tasks (compactor, flusher, GC, batch-writer); if one were left open,
//! those tasks would keep running and skew the next measurement.
//!
//! Functions, each isolating one variable:
//!   slatedb_native        — today: caller runs on a multi-thread runtime, the batch-writer task
//!                           lands on a worker thread, so every write costs a cross-thread wake-up.
//!   slatedb_native_current_thread
//!                         — the whole DB on one current-thread runtime. The obvious fix, and it is
//!                           *slower*: that runtime's single thread also drives the IO/timer driver,
//!                           and `block_on`'s loop parks every iteration, so each write pays a
//!                           `kevent` syscall.
//!   slatedb_native_split_io
//!                         — only the batch-writer moved onto the caller's current-thread runtime;
//!                           flusher/compactor/GC/monitor stay on the multi-thread runtime. Still
//!                           built with `enable_all()`. Shows that background components competing
//!                           for the thread were never the problem: the number does not move.
//!   slatedb_native_split_runtime
//!                         — the same split, but the caller's runtime is built with `enable_time()`
//!                           instead of `enable_all()`: timer yes, IO driver no. The `kevent` goes
//!                           away. This is the fix.
//!   rocksdb               — RocksDB's synchronous put, inline on the calling thread — reference.
//!
//! The timer cannot be dropped too: `MonotonicClock::now()` sleeps on clock skew and
//! `monitor_first_write` sleeps on the first durable write, both on the caller's runtime.
//!
//! Storage differs by design: SlateDB targets an object store (here a LocalFileSystem dir), RocksDB
//! a local dir. Both hit the local filesystem here; the media cannot be made identical.
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::runtime::{Builder, Handle, Runtime};

const KEYS: u64 = 100;
const MEMTABLE: usize = 60 * 1024 * 1024; // 60 MiB

fn slatedb_settings() -> slatedb::config::Settings {
    let mut s = slatedb::config::Settings::default();
    s.wal_enabled = false;
    s.l0_sst_size_bytes = MEMTABLE;
    s.max_unflushed_bytes = 2 * MEMTABLE;
    s
}

/// Fresh SlateDB at `dir`: clear any prior data, then open. When `write_runtime` is set, only the
/// batch-writer task moves there; every background component stays on the runtime we build on.
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

fn write_benches(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let base = std::env::temp_dir().join("write-bench");
    let wopts = slatedb::config::WriteOptions { await_durable: false, ..Default::default() };
    let popts = slatedb::config::PutOptions::default();
    let mut group = c.benchmark_group("write");

    // --- today: batch-writer on a worker thread, one cross-thread round-trip per write ---
    {
        let db = rt.block_on(fresh_slatedb(&base.join("native"), None));
        let counter = AtomicU64::new(0);
        group.bench_function("slatedb_native", |b| {
            b.to_async(&rt).iter(|| async {
                let i = counter.fetch_add(1, Ordering::Relaxed);
                let (key, val) = ((i % KEYS).to_be_bytes(), i.to_be_bytes());
                db.put_with_options(&key, &val, &popts, &wopts).await.unwrap();
                black_box(());
            });
        });
        rt.block_on(db.close()).unwrap(); // teardown: stop background tasks before the next section
    }

    // --- whole DB on one current-thread runtime: no cross-thread wake-up, but a `kevent` per write
    // from the IO driver this thread now also has to drive ---
    {
        let ct = Builder::new_current_thread().enable_all().build().unwrap();
        let db = ct.block_on(fresh_slatedb(&base.join("native_current_thread"), None));
        let counter = AtomicU64::new(0);
        group.bench_function("slatedb_native_current_thread", |b| {
            b.to_async(&ct).iter(|| async {
                let i = counter.fetch_add(1, Ordering::Relaxed);
                let (key, val) = ((i % KEYS).to_be_bytes(), i.to_be_bytes());
                db.put_with_options(&key, &val, &popts, &wopts).await.unwrap();
                black_box(());
            });
        });
        ct.block_on(db.close()).unwrap();
    }

    // --- only the batch-writer split off, IO driver still enabled: isolates that background
    // components sharing the thread were not the cost ---
    {
        let ct = Builder::new_current_thread().enable_all().build().unwrap();
        let db = rt.block_on(fresh_slatedb(&base.join("native_split_io"), Some(ct.handle().clone())));
        let counter = AtomicU64::new(0);
        group.bench_function("slatedb_native_split_io", |b| {
            b.to_async(&ct).iter(|| async {
                let i = counter.fetch_add(1, Ordering::Relaxed);
                let (key, val) = ((i % KEYS).to_be_bytes(), i.to_be_bytes());
                db.put_with_options(&key, &val, &popts, &wopts).await.unwrap();
                black_box(());
            });
        });
        ct.block_on(db.close()).unwrap();
    }

    // --- the fix: same split, caller's runtime built with `enable_time()` and no IO driver, so the
    // scheduler's per-loop park is a thread park instead of a `kevent` syscall ---
    {
        let ct = Builder::new_current_thread().enable_time().build().unwrap();
        let db = rt.block_on(fresh_slatedb(&base.join("native_split"), Some(ct.handle().clone())));
        let counter = AtomicU64::new(0);
        group.bench_function("slatedb_native_split_runtime", |b| {
            b.to_async(&ct).iter(|| async {
                let i = counter.fetch_add(1, Ordering::Relaxed);
                let (key, val) = ((i % KEYS).to_be_bytes(), i.to_be_bytes());
                db.put_with_options(&key, &val, &popts, &wopts).await.unwrap();
                black_box(());
            });
        });
        ct.block_on(db.close()).unwrap();
    }

    // --- RocksDB: synchronous inline put (dropped at end of scope) ---
    {
        let rdir = base.join("rocksdb");
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.set_write_buffer_size(MEMTABLE);
        let _ = rocksdb::DB::destroy(&opts, &rdir);
        let rdb = rocksdb::DB::open(&opts, &rdir).unwrap();
        let mut rwopts = rocksdb::WriteOptions::default();
        rwopts.set_sync(false);
        rwopts.disable_wal(true);
        let counter = AtomicU64::new(0);
        group.bench_function("rocksdb", |b| {
            b.iter(|| {
                let i = counter.fetch_add(1, Ordering::Relaxed);
                let (key, val) = ((i % KEYS).to_be_bytes(), i.to_be_bytes());
                rdb.put_opt(black_box(key), black_box(val), &rwopts).unwrap();
            });
        });
    }

    group.finish();
}

criterion_group!(benches, write_benches);
criterion_main!(benches);
