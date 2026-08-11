// Snapshot polling helpers for SlateDB metrics.
//
// SlateDB's Rust core pushes every metric update into a `MetricsRecorder`, and
// those updates happen on SlateDB's own background threads. A recorder
// implemented in JavaScript therefore turns each update into a synchronous
// cross-thread call into the Node event loop, which can stall (and, when the
// event loop is itself waiting on SlateDB, deadlock) the whole process.
//
// The supported way to collect metrics from Node is to keep the recorder on the
// Rust side (`DefaultMetricsRecorder`, backed by atomics) and to pull values
// from JavaScript on an interval. The helpers here do the pulling and reshape
// the generated snapshot records into plain objects that are easy to hand to an
// exporter.

/**
 * Key that the generated bindings use for the payload of a single-field enum
 * variant, e.g. `metric.value[""]` for `MetricValue::Counter(u64)`.
 */
const VARIANT_PAYLOAD_KEY = "";

const DEFAULT_INTERVAL_MS = 10_000;

const METRIC_TYPES = new Map([
  ["Counter", "counter"],
  ["Gauge", "gauge"],
  ["UpDownCounter", "upDownCounter"],
  ["Histogram", "histogram"],
]);

/**
 * Converts one snapshot record into a plain object.
 *
 * Labels become a `{ key: value }` object, the variant tag becomes a lowercase
 * `type` string, and the payload moves to `value`. Integer values stay
 * `BigInt`, matching the `u64`/`i64` metrics they come from.
 */
export function normalizeMetric(metric) {
  const tag = metric?.value?.tag;
  const type = METRIC_TYPES.get(tag);
  if (type == null) {
    throw new TypeError(`unknown metric value tag: ${String(tag)}`);
  }

  const payload = metric.value[VARIANT_PAYLOAD_KEY];
  return {
    name: metric.name,
    labels: normalizeLabels(metric.labels),
    description: metric.description ?? "",
    type,
    value: type === "histogram" ? normalizeHistogram(payload) : payload,
  };
}

/**
 * Converts a whole `DefaultMetricsRecorder.snapshot()` result with
 * {@link normalizeMetric}.
 */
export function normalizeSnapshot(metrics) {
  return metrics.map(normalizeMetric);
}

/**
 * Takes an immediate snapshot of `recorder` and normalizes it.
 */
export function snapshotMetrics(recorder) {
  return normalizeSnapshot(recorder.snapshot());
}

/**
 * Polls `recorder` every `intervalMs` and passes each snapshot to `onSnapshot`.
 *
 * The interval timer is unref'd, so it never keeps the process alive on its
 * own. Call `stop()` on the returned handle before disposing the recorder.
 *
 * If a scheduled poll throws — from `snapshot()` or from `onSnapshot` — polling
 * stops and the error goes to `onError`, or is rethrown when no `onError` is
 * given. Errors from a manual `poll()` are thrown to its caller instead and
 * leave the timer running.
 *
 * @param recorder `DefaultMetricsRecorder` shared with the `Db`/`DbReader`.
 * @param options.onSnapshot Receives each snapshot; required.
 * @param options.intervalMs Poll interval in milliseconds; defaults to 10s.
 * @param options.onError Receives the error that stopped polling.
 * @param options.normalize Pass `false` to receive raw snapshot records.
 * @returns Handle with `poll()` and `stop()`.
 */
export function pollMetrics(
  recorder,
  { onSnapshot, intervalMs = DEFAULT_INTERVAL_MS, onError, normalize = true } = {},
) {
  if (typeof recorder?.snapshot !== "function") {
    throw new TypeError("recorder must expose a snapshot() method");
  }
  if (typeof onSnapshot !== "function") {
    throw new TypeError("onSnapshot must be a function");
  }
  if (onError != null && typeof onError !== "function") {
    throw new TypeError("onError must be a function");
  }
  if (!Number.isFinite(intervalMs) || intervalMs <= 0) {
    throw new RangeError("intervalMs must be a positive finite number");
  }

  let timer = undefined;

  const poll = () => {
    const metrics = recorder.snapshot();
    onSnapshot(normalize ? normalizeSnapshot(metrics) : metrics);
  };

  const stop = () => {
    if (timer != null) {
      clearInterval(timer);
      timer = undefined;
    }
  };

  timer = setInterval(() => {
    try {
      poll();
    } catch (error) {
      stop();
      if (onError == null) {
        throw error;
      }
      onError(error);
    }
  }, intervalMs);
  timer.unref?.();

  return { poll, stop };
}

function normalizeLabels(labels) {
  const normalized = {};
  for (const label of labels ?? []) {
    normalized[label.key] = label.value;
  }
  return normalized;
}

function normalizeHistogram(histogram) {
  return {
    count: histogram.count,
    sum: histogram.sum,
    min: histogram.min,
    max: histogram.max,
    boundaries: histogram.boundaries,
    bucketCounts: histogram.bucket_counts,
  };
}
