//! Criterion benchmark isolating the cross-thread request/reply round-trip that SlateDB's write
//! path performs per write (`write_notifier.send(..)` + `rx.await` on a Tokio runtime). The payload
//! is a trivial `+1`, so the measured time is the round-trip itself, not the work.
//!
//! Two functions:
//!   same_thread   — a plain `+1` on the calling thread (baseline: the work itself is ~ns).
//!   tokio_oneshot — send a value to a resident worker task on a multi-thread Tokio runtime and
//!                   block on its `oneshot` reply — the same API shape as SlateDB's write path.
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use tokio::runtime::Runtime;
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
    let rt = Runtime::new().unwrap();

    // Resident worker task: receive (value, reply), send back value+1. Mirrors the batch-writer.
    type Req = (u64, oneshot::Sender<u64>);
    let (req_tx, mut req_rx) = mpsc::unbounded_channel::<Req>();
    rt.spawn(async move {
        while let Some((v, done)) = req_rx.recv().await {
            let _ = done.send(v.wrapping_add(1));
        }
    });

    let mut n: u64 = 0;
    c.bench_function("tokio_oneshot", |b| {
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

criterion_group!(benches, same_thread, tokio_oneshot);
criterion_main!(benches);
