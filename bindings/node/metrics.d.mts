import type { Metric } from "./index.js";

/** Metric kind, derived from the snapshot record's value variant. */
export type MetricType = "counter" | "gauge" | "upDownCounter" | "histogram";

/** Histogram payload with camelCase fields. */
export interface NormalizedHistogram {
  count: bigint | number;
  sum: number;
  min: number;
  max: number;
  boundaries: Array<number>;
  bucketCounts: Array<bigint | number>;
}

interface NormalizedMetricBase {
  name: string;
  labels: Record<string, string>;
  description: string;
}

/** Counter, gauge, or up/down counter metric. */
export interface NormalizedScalarMetric extends NormalizedMetricBase {
  type: "counter" | "gauge" | "upDownCounter";
  value: bigint | number;
}

/** Histogram metric. */
export interface NormalizedHistogramMetric extends NormalizedMetricBase {
  type: "histogram";
  value: NormalizedHistogram;
}

/** One metric reshaped into a plain object. */
export type NormalizedMetric = NormalizedScalarMetric | NormalizedHistogramMetric;

/** Anything that can produce a metric snapshot, e.g. `DefaultMetricsRecorder`. */
export interface MetricsSnapshotSource {
  snapshot(): Array<Metric>;
}

/** Handle returned by {@link pollMetrics}. */
export interface MetricsPoller {
  /** Takes a snapshot now; throws whatever `snapshot()` throws. */
  poll(): void;
  /** Stops polling. Idempotent. */
  stop(): void;
}

interface PollMetricsOptionsBase {
  /** Poll interval in milliseconds. Defaults to 10000. */
  intervalMs?: number;
  /** Receives the error that stopped polling. */
  onError?: (error: unknown) => void;
}

export interface NormalizedPollMetricsOptions extends PollMetricsOptionsBase {
  onSnapshot: (metrics: Array<NormalizedMetric>) => void;
  normalize?: true;
}

export interface RawPollMetricsOptions extends PollMetricsOptionsBase {
  onSnapshot: (metrics: Array<Metric>) => void;
  normalize: false;
}

export type PollMetricsOptions = NormalizedPollMetricsOptions | RawPollMetricsOptions;

export declare function normalizeMetric(metric: Metric): NormalizedMetric;

export declare function normalizeSnapshot(metrics: Array<Metric>): Array<NormalizedMetric>;

export declare function snapshotMetrics(
  recorder: MetricsSnapshotSource,
): Array<NormalizedMetric>;

export declare function pollMetrics(
  recorder: MetricsSnapshotSource,
  options: PollMetricsOptions,
): MetricsPoller;
