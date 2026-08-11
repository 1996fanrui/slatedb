import assert from "node:assert/strict";
import test from "node:test";

import { DefaultMetricsRecorder } from "../index.js";
import { normalizeMetric, pollMetrics, snapshotMetrics } from "../metrics.mjs";
import {
  bytes,
  createCleanup,
  newMemoryStore,
  openDb,
  openReader,
  putOptions,
  waitUntil,
  writeOptions,
} from "./support.mjs";

const DB_REQUEST_COUNT = "slatedb.db.request_count";
const DB_WRITE_OPS = "slatedb.db.write_ops";

function toBigInt(value) {
  return typeof value === "bigint" ? value : BigInt(value);
}

function findMetric(metrics, name, labels = {}) {
  return metrics.find(
    (metric) =>
      metric.name === name &&
      Object.entries(labels).every(([key, value]) => metric.labels[key] === value),
  );
}

test("default metrics recorder snapshots and lookups", async (t) => {
  const cleanup = createCleanup(t);
  const recorder = cleanup.track(new DefaultMetricsRecorder(), { shutdown: false });

  recorder.register_counter("test.counter", "counter", []).increment(3);
  recorder.register_gauge("test.gauge", "gauge", []).set(-7);
  const upDownCounter = recorder.register_up_down_counter("test.up-down", "up-down", []);
  upDownCounter.increment(5);
  upDownCounter.increment(-2);
  const histogram = recorder.register_histogram("test.histogram", "histogram", [], [1, 2]);
  histogram.record(1.5);
  histogram.record(3);

  const metricsByName = recorder.metrics_by_name("test.counter");
  assert.equal(metricsByName.length, 1);
  assert.equal(metricsByName[0].value.tag, "Counter");
  assert.equal(toBigInt(metricsByName[0].value[""]), 3n);

  const histogramMetric = recorder.metric_by_name_and_labels("test.histogram", []);
  assert.notEqual(histogramMetric, undefined);
  assert.equal(histogramMetric.value.tag, "Histogram");
  assert.equal(toBigInt(histogramMetric.value[""].count), 2n);
  assert.equal(histogramMetric.value[""].sum, 4.5);
  assert.deepEqual(histogramMetric.value[""].boundaries, [1, 2]);
  assert.deepEqual(
    histogramMetric.value[""].bucket_counts.map(toBigInt),
    [0n, 1n, 1n],
  );

  const snapshot = recorder.snapshot();
  assert.ok(snapshot.length >= 4);
});

test("normalized snapshots expose plain metric objects", async (t) => {
  const cleanup = createCleanup(t);
  const recorder = cleanup.track(new DefaultMetricsRecorder(), { shutdown: false });

  recorder
    .register_counter("test.counter", "counter", [{ key: "op", value: "get" }])
    .increment(3);
  recorder.register_gauge("test.gauge", "gauge", []).set(-7);
  const upDownCounter = recorder.register_up_down_counter("test.up-down", "up-down", []);
  upDownCounter.increment(5);
  upDownCounter.increment(-2);
  const histogram = recorder.register_histogram("test.histogram", "histogram", [], [1, 2]);
  histogram.record(1.5);
  histogram.record(3);

  const metrics = snapshotMetrics(recorder);

  const counter = findMetric(metrics, "test.counter", { op: "get" });
  assert.deepEqual(counter.labels, { op: "get" });
  assert.equal(counter.description, "counter");
  assert.equal(counter.type, "counter");
  assert.equal(toBigInt(counter.value), 3n);

  assert.equal(findMetric(metrics, "test.gauge").type, "gauge");
  assert.equal(toBigInt(findMetric(metrics, "test.gauge").value), -7n);

  const upDown = findMetric(metrics, "test.up-down");
  assert.equal(upDown.type, "upDownCounter");
  assert.equal(toBigInt(upDown.value), 3n);

  const histogramMetric = findMetric(metrics, "test.histogram");
  assert.equal(histogramMetric.type, "histogram");
  assert.equal(toBigInt(histogramMetric.value.count), 2n);
  assert.equal(histogramMetric.value.sum, 4.5);
  assert.deepEqual(histogramMetric.value.boundaries, [1, 2]);
  assert.deepEqual(histogramMetric.value.bucketCounts.map(toBigInt), [0n, 1n, 1n]);
});

test("normalizeMetric rejects unknown metric values", () => {
  assert.throws(
    () => normalizeMetric({ name: "test.unknown", labels: [], value: { tag: "Summary" } }),
    TypeError,
  );
});

test("db builder records into the default metrics recorder", async (t) => {
  const cleanup = createCleanup(t);
  const store = cleanup.track(newMemoryStore());
  const recorder = cleanup.track(new DefaultMetricsRecorder(), { shutdown: false });
  const db = await openDb(store, {
    cleanup,
    configure(builder) {
      builder.with_default_metrics_recorder(recorder);
    },
  });

  await db.put(bytes("k1"), bytes("v1"));
  await db.put(bytes("k2"), bytes("v2"));

  const writeOps = findMetric(snapshotMetrics(recorder), DB_WRITE_OPS);
  assert.notEqual(writeOps, undefined);
  assert.equal(toBigInt(writeOps.value), 2n);
});

// Registering or updating a metric from JavaScript blocks a SlateDB background
// thread on the event loop, which is how https://github.com/slatedb/slatedb/issues/2004
// deadlocked. with_default_metrics_recorder must keep all of it on the Rust side.
test("the default recorder never calls back into JavaScript", async (t) => {
  const cleanup = createCleanup(t);
  const store = cleanup.track(newMemoryStore());
  const recorder = cleanup.track(new DefaultMetricsRecorder(), { shutdown: false });

  let jsCalls = 0;
  const originals = new Map();
  for (const name of [
    "register_counter",
    "register_gauge",
    "register_up_down_counter",
    "register_histogram",
  ]) {
    const original = DefaultMetricsRecorder.prototype[name];
    originals.set(name, original);
    DefaultMetricsRecorder.prototype[name] = function (...args) {
      jsCalls += 1;
      return original.apply(this, args);
    };
  }
  t.after(() => {
    for (const [name, original] of originals) {
      DefaultMetricsRecorder.prototype[name] = original;
    }
  });

  const db = await openDb(store, {
    cleanup,
    configure(builder) {
      builder.with_default_metrics_recorder(recorder);
    },
  });
  await db.put(bytes("k1"), bytes("v1"));
  await db.flush();

  assert.equal(jsCalls, 0, "metric registration should stay on the Rust side");
  assert.ok(snapshotMetrics(recorder).length > 0, "the recorder should hold metrics");
});

test("polling observes metrics while the db is running", async (t) => {
  const cleanup = createCleanup(t);
  const store = cleanup.track(newMemoryStore());
  const recorder = cleanup.track(new DefaultMetricsRecorder(), { shutdown: false });
  const db = await openDb(store, {
    cleanup,
    configure(builder) {
      builder.with_default_metrics_recorder(recorder);
    },
  });

  let latest = undefined;
  const poller = pollMetrics(recorder, {
    intervalMs: 10,
    onSnapshot(metrics) {
      latest = metrics;
    },
  });

  try {
    await db.put(bytes("k1"), bytes("v1"));
    await db.put(bytes("k2"), bytes("v2"));

    await waitUntil(() => {
      const writeOps = findMetric(latest ?? [], DB_WRITE_OPS);
      return writeOps != null && toBigInt(writeOps.value) === 2n;
    });
  } finally {
    poller.stop();
  }

  latest = undefined;
  await new Promise((resolve) => setTimeout(resolve, 50));
  assert.equal(latest, undefined, "stop() should end polling");
});

test("poll() delivers a snapshot without waiting for the interval", async (t) => {
  const cleanup = createCleanup(t);
  const recorder = cleanup.track(new DefaultMetricsRecorder(), { shutdown: false });
  recorder.register_counter("test.counter", "counter", []).increment(4);

  const snapshots = [];
  const poller = pollMetrics(recorder, {
    intervalMs: 60_000,
    onSnapshot(metrics) {
      snapshots.push(metrics);
    },
  });

  try {
    poller.poll();
  } finally {
    poller.stop();
  }

  assert.equal(snapshots.length, 1);
  assert.equal(toBigInt(findMetric(snapshots[0], "test.counter").value), 4n);
});

test("polling stops and reports when a snapshot fails", async () => {
  const failure = new Error("recorder is closed");
  const recorder = {
    calls: 0,
    snapshot() {
      this.calls += 1;
      throw failure;
    },
  };

  const errors = [];
  pollMetrics(recorder, {
    intervalMs: 5,
    onSnapshot() {
      assert.fail("onSnapshot should not run for a failed snapshot");
    },
    onError(error) {
      errors.push(error);
    },
  });

  await waitUntil(() => errors.length === 1);
  await new Promise((resolve) => setTimeout(resolve, 30));

  assert.deepEqual(errors, [failure]);
  assert.equal(recorder.calls, 1, "a failing poll should not be retried");
});

test("pollMetrics validates its arguments", () => {
  const recorder = { snapshot: () => [] };

  assert.throws(() => pollMetrics({}, { onSnapshot() {} }), TypeError);
  assert.throws(() => pollMetrics(recorder, {}), TypeError);
  assert.throws(() => pollMetrics(recorder, { onSnapshot() {}, onError: 7 }), TypeError);
  assert.throws(
    () => pollMetrics(recorder, { onSnapshot() {}, intervalMs: 0 }),
    RangeError,
  );
});

test("reader builder accepts default metrics recorder", async (t) => {
  const cleanup = createCleanup(t);
  const store = cleanup.track(newMemoryStore());
  const db = await openDb(store, { cleanup });

  await db.put_with_options(
    bytes("key1"),
    bytes("value1"),
    putOptions(),
    writeOptions(false),
  );
  await db.flush();

  const recorder = cleanup.track(new DefaultMetricsRecorder(), { shutdown: false });
  const reader = await openReader(store, {
    cleanup,
    configure(builder) {
      builder.with_default_metrics_recorder(recorder);
    },
  });

  assert.deepEqual(await reader.get(bytes("key1")), bytes("value1"));

  const requestCount = findMetric(snapshotMetrics(recorder), DB_REQUEST_COUNT, {
    op: "get",
  });
  assert.notEqual(requestCount, undefined);
  assert.equal(toBigInt(requestCount.value), 1n);
});
