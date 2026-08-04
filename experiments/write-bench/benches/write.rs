//! Single-threaded write benchmark comparing SlateDB's write paths against RocksDB, under one
//! aligned config so the numbers are directly comparable.
//!
//! Workload (all functions): 100 keys overwritten in a tight loop, single writer thread, WAL
//! disabled, no per-write durability, ~60 MiB memtable, local filesystem. After warm-up the working
//! set is in steady state (~100 versioned keys in memory), which is what Criterion assumes.
//!
//! Isolation: each function opens its OWN fresh database, and — importantly — the previous one is
//! fully closed before the next starts. A SlateDB `Db` runs background tasks (compactor, flusher,
//! GC, batch-writer) on the shared runtime; if it were left open, those tasks would keep running
//! and skew the next function's measurement. So each SlateDB section does open -> bench -> close.
//! (This is the "setup / teardown" the benchmark needs.)
//!
//! Functions:
//!   slatedb_native    — Db::put_with_options: current path (dispatch to the batch-writer task +
//!                       await a oneshot reply → one cross-thread round-trip per write).
//!   slatedb_local     — Db::put_local: an experimental inline write on the calling thread.
//!   slatedb_local_cas — slatedb_local plus one CAS. The CAS is NOT part of SlateDB; it is added
//!                       only here to model the contention check a leader/group-commit design would
//!                       do on its fast path. local vs local_cas shows that check is ~free.
//!   rocksdb           — RocksDB's synchronous put (inline on the calling thread) — reference point.
//!
//! Storage differs by design: SlateDB targets an object store (here a LocalFileSystem dir), RocksDB
//! a local dir. Both hit the local filesystem here; the media cannot be made identical.
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::runtime::Runtime;

const KEYS: u64 = 100;
const MEMTABLE: usize = 60 * 1024 * 1024; // 60 MiB

fn slatedb_settings() -> slatedb::config::Settings {
    let mut s = slatedb::config::Settings::default();
    s.wal_enabled = false;
    s.l0_sst_size_bytes = MEMTABLE;
    s.max_unflushed_bytes = 2 * MEMTABLE;
    s
}

/// Fresh SlateDB at `dir`: clear any prior data, then open.
async fn fresh_slatedb(dir: &std::path::Path) -> slatedb::Db {
    use slatedb::object_store::{local::LocalFileSystem, ObjectStore};
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let object_store: Arc<dyn ObjectStore> = Arc::new(LocalFileSystem::new_with_prefix(dir).unwrap());
    slatedb::Db::builder("bench", object_store)
        .with_settings(slatedb_settings())
        .build()
        .await
        .unwrap()
}

fn write_benches(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let base = std::env::temp_dir().join("write-bench");
    let wopts = slatedb::config::WriteOptions { await_durable: false, ..Default::default() };
    let popts = slatedb::config::PutOptions::default();
    let mut group = c.benchmark_group("write");

    // --- SlateDB: current cross-thread path ---
    {
        let db = rt.block_on(fresh_slatedb(&base.join("native")));
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

    // --- SlateDB: inline write ---
    {
        let db = rt.block_on(fresh_slatedb(&base.join("local")));
        let counter = AtomicU64::new(0);
        group.bench_function("slatedb_local", |b| {
            b.to_async(&rt).iter(|| async {
                let i = counter.fetch_add(1, Ordering::Relaxed);
                let (key, val) = ((i % KEYS).to_be_bytes(), i.to_be_bytes());
                db.put_local(&key, &val).await.unwrap();
                black_box(());
            });
        });
        rt.block_on(db.close()).unwrap();
    }

    // --- SlateDB: inline write + CAS (models a leader's contention check) ---
    {
        let db = rt.block_on(fresh_slatedb(&base.join("local_cas")));
        let counter = AtomicU64::new(0);
        let writing = AtomicBool::new(false);
        group.bench_function("slatedb_local_cas", |b| {
            b.to_async(&rt).iter(|| async {
                let i = counter.fetch_add(1, Ordering::Relaxed);
                let (key, val) = ((i % KEYS).to_be_bytes(), i.to_be_bytes());
                writing
                    .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                    .unwrap();
                db.put_local(&key, &val).await.unwrap();
                writing.store(false, Ordering::Release);
                black_box(());
            });
        });
        rt.block_on(db.close()).unwrap();
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
