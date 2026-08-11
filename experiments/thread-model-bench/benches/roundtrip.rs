//! Criterion benchmark isolating the cross-thread request/reply round-trip that SlateDB's write
//! path performs per write (`write_notifier.send(..)` + `rx.await` on a Tokio runtime). The payload
//! is a trivial `+1`, so the measured time is the round-trip itself, not the work.
//!
//! Three functions:
//!   same_thread   — a plain `+1` on the calling thread (baseline: the work itself is ~ns).
//!   tokio_oneshot — send a value to a resident worker task on a multi-thread Tokio runtime and
//!                   block on its `oneshot` reply — the same API shape as SlateDB's write path.
//!   tokio_oneshot_current_thread — the same round-trip on a current-thread runtime, where caller
//!                   and worker share one OS thread: isolates how much of the cost is the
//!                   cross-thread wake-up rather than the hand-off itself.
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc;
use tokio::sync::oneshot;

fn same_thread(c: &mut Criterion) {
    let mut n: u64 = 0;
    c.bench_function("same_thread", |b| {
        b.iter(|| {
            n = black_box(n).wrapping_add(1);
            black_box(n)
        })
    });
}

fn tokio_oneshot(c: &mut Criterion) {
    roundtrip(c, "tokio_oneshot", Runtime::new().unwrap());
}

fn tokio_oneshot_current_thread(c: &mut Criterion) {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    roundtrip(c, "tokio_oneshot_current_thread", rt);
}

fn roundtrip(c: &mut Criterion, name: &str, rt: Runtime) {
    // Resident worker task: receive (value, reply), send back value+1. Mirrors the batch-writer.
    type Req = (u64, oneshot::Sender<u64>);
    let (req_tx, mut req_rx) = mpsc::unbounded_channel::<Req>();
    rt.spawn(async move {
        while let Some((v, done)) = req_rx.recv().await {
            let _ = done.send(v.wrapping_add(1));
        }
    });

    let mut n: u64 = 0;
    c.bench_function(name, |b| {
        b.to_async(&rt).iter(|| {
            let req_tx = req_tx.clone();
            let v = n;
            async move {
                let (done_tx, done_rx) = oneshot::channel::<u64>();
                req_tx.send((v, done_tx)).unwrap(); // write_notifier.send(batch_msg)
                done_rx.await.unwrap() // rx.await
            }
        });
        n = n.wrapping_add(1);
    });
}

criterion_group!(
    benches,
    same_thread,
    tokio_oneshot,
    tokio_oneshot_current_thread
);
criterion_main!(benches);
