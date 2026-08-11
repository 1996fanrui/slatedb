# SlateDB Node Binding

`bindings/node` contains the official Node.js package for SlateDB.

## Install

Package:

```text
@slatedb/uniffi
```

Requirements:

- Node.js 20 or newer

Install from npm:

```bash
npm install @slatedb/uniffi
```

## API Model

- `ObjectStore.resolve(...)` opens an object store from a URL such as `memory:///` or `file:///...`
- `DbBuilder` opens a writable database and `DbReaderBuilder` opens a read-only reader
- keys and values are binary; pass `Buffer` or `Uint8Array`
- most database operations are async and should be awaited
- builders are single-use; `Db` and `DbReader` stay open until `shutdown()` resolves
- native-backed handles also expose `dispose()` for deterministic cleanup after `shutdown()` or when abandoning a builder

## Quick Start

```js
import assert from "node:assert/strict";
import { DbBuilder, ObjectStore } from "@slatedb/uniffi";

async function main() {
  const store = ObjectStore.resolve("memory:///");
  let db;

  try {
    const builder = new DbBuilder("demo-db", store);
    try {
      db = await builder.build();
    } finally {
      builder.dispose();
    }

    const key = Buffer.from("hello");
    const value = Buffer.from("world");

    await db.put(key, value);

    const read = await db.get(key);
    assert.deepEqual(read, value);

    console.log(Buffer.from(read).toString("utf8"));
  } finally {
    if (db != null) {
      await db.shutdown();
      db.dispose();
    }
    store.dispose();
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
```

Replace `memory:///` with any object store URL supported by Rust's [`object_store`](https://docs.rs/object_store/latest/object_store/fn.parse_url_opts.html) crate.

## Metrics

Metrics are collected by pulling, not by pushing: keep the built-in
`DefaultMetricsRecorder` on the Rust side, attach it with
`with_default_metrics_recorder(...)`, and read values out of it from JavaScript
whenever you want to export them.

- `DbBuilder.with_default_metrics_recorder(...)`
- `DbReaderBuilder.with_default_metrics_recorder(...)`
- `DefaultMetricsRecorder.snapshot()`
- `DefaultMetricsRecorder.metrics_by_name(...)`
- `DefaultMetricsRecorder.metric_by_name_and_labels(...)`

`@slatedb/uniffi/metrics` adds the polling loop and reshapes snapshot records
into plain objects (`{ name, labels, description, type, value }`, integers as
`BigInt`):

- `pollMetrics(recorder, options)` polls on an interval and returns `{ poll(), stop() }`
- `snapshotMetrics(recorder)` takes one normalized snapshot
- `normalizeSnapshot(metrics)` / `normalizeMetric(metric)` normalize records you already have

```js
import { DbBuilder, DefaultMetricsRecorder, ObjectStore } from "@slatedb/uniffi";
import { pollMetrics } from "@slatedb/uniffi/metrics";

const store = ObjectStore.resolve("memory:///");
const recorder = new DefaultMetricsRecorder();
const builder = new DbBuilder("metrics-demo", store);

try {
  builder.with_default_metrics_recorder(recorder);
  const db = await builder.build();
  const poller = pollMetrics(recorder, {
    intervalMs: 10_000,
    onSnapshot(metrics) {
      for (const metric of metrics) {
        console.log(metric.name, metric.labels, metric.type, metric.value);
      }
    },
    onError(error) {
      console.error("metrics polling stopped", error);
    },
  });

  try {
    await db.put(Buffer.from("hello"), Buffer.from("world"));
  } finally {
    poller.stop();
    await db.shutdown();
    db.dispose();
  }
} finally {
  builder.dispose();
  recorder.dispose();
  store.dispose();
}
```

Stop the poller before disposing the recorder. The interval timer is unref'd, so
it never keeps the process alive on its own.

### Why not a JavaScript recorder?

`with_metrics_recorder(...)` accepts any object implementing the
`MetricsRecorder` interface, but implementing one in JavaScript is not
recommended. SlateDB registers and updates metrics from its own background
threads, so each call becomes a synchronous cross-thread call into the Node
event loop. That is far more expensive than polling, and it can deadlock: if the
event loop is waiting on a SlateDB operation while SlateDB is waiting for the
event loop to service a metric update, neither side makes progress (see
[#2004](https://github.com/slatedb/slatedb/issues/2004)).

`with_default_metrics_recorder(...)` avoids this entirely — the recorder is
attached as a Rust object, so no metric registration or update crosses back into
JavaScript.

## Local Development

The package is generated from the UniFFI `slatedb-uniffi` cdylib using [`uniffi-bindgen-node-js`](https://crates.io/crates/uniffi-bindgen-node-js).

You only need these tools when regenerating bindings, running tests from this repository, or packing the npm artifact locally:

- Node.js 20 or newer
- Rust toolchain for this repository
- `uniffi-bindgen-node-js` on `PATH`

Install the generator with:

```bash
cargo install uniffi-bindgen-node-js --version 0.0.13
```

Install the package dependency used by the generated bindings with:

```bash
npm --prefix bindings/node install
```

### Regenerate Bindings

From the repository root:

```bash
npm --prefix bindings/node run build
```

This command:

1. builds the host `slatedb-uniffi` library
2. runs `uniffi-bindgen-node-js`
3. copies the generated package files into `bindings/node`
4. stages the host native library under `bindings/node/prebuilds/<target>/`

Generated API files are written into `bindings/node` and are not committed. `package.json`, `build.mjs`, `metrics.mjs`, `metrics.d.mts`, and this `README.md` are maintained by hand.

### Run Tests

From the repository root:

```bash
npm --prefix bindings/node test
```

The test script rebuilds the package and then runs `node --test` inside `bindings/node`.

### Reproduce The PR CI Flow

From the repository root, this mirrors the Node validation done in `.github/workflows/pr.yaml`:

```bash
npm --prefix bindings/node ci
npm --prefix bindings/node run build
(cd bindings/node && node --test)
git diff --exit-code -- bindings/node
rm -rf /tmp/slatedb-node-pack
mkdir -p /tmp/slatedb-node-pack
(cd bindings/node && npm pack --pack-destination /tmp/slatedb-node-pack)
TARBALL="$(find /tmp/slatedb-node-pack -maxdepth 1 -name '*.tgz' | head -n 1)"
test -n "${TARBALL}"
tar -tf "${TARBALL}" | grep -Fx 'package/index.js'
tar -tf "${TARBALL}" | grep -Fx 'package/index.d.ts'
tar -tf "${TARBALL}" | grep -Fx 'package/slatedb.js'
tar -tf "${TARBALL}" | grep -Fx 'package/slatedb.d.ts'
tar -tf "${TARBALL}" | grep -Fx 'package/slatedb-ffi.js'
tar -tf "${TARBALL}" | grep -Fx 'package/slatedb-ffi.d.ts'
tar -tf "${TARBALL}" | grep -Fx 'package/metrics.mjs'
tar -tf "${TARBALL}" | grep -Fx 'package/metrics.d.mts'
tar -tf "${TARBALL}" | grep -Fx 'package/runtime/ffi-types.js'
tar -tf "${TARBALL}" | grep -Fx 'package/prebuilds/linux-x64-gnu/libslatedb_uniffi.so'
```

## Packaging And Runtime Notes

The published `@slatedb/uniffi` tarball contains generated JavaScript and TypeScript bindings plus bundled native libraries. Consumers install one npm package; there is no separate Rust build step or native download in normal usage.

Writable databases and readers enter a dedicated multi-threaded Tokio runtime while opening so SlateDB's long-lived background tasks capture a multi-threaded Tokio handle. Foreground binding calls are still awaited through the normal JavaScript async API. By default, the runtime uses the host's available parallelism. Set `SLATEDB_UNIFFI_RUNTIME_THREADS` to a positive integer before the first `DbBuilder.build()` or `DbReaderBuilder.build()` call to override the worker thread count for the lifetime of the process.

At runtime, the package loads the native library that matches the current host from `prebuilds/<target>/`. The published package currently includes:

- `linux-x64-gnu`
- `linux-arm64-gnu`
- `darwin-x64`
- `darwin-arm64`
- `win32-x64`
- `win32-arm64`

Linux musl targets are not packaged today. Local builds stage only the host native library; release builds stage all supported targets into the published npm package.

When assembling a release package from a prebuilt native directory, run:

```bash
npm --prefix bindings/node run build -- --prebuilt-dir <dir>
```

The generated API files stay the same; only the packaged native libraries change between local and release builds.
