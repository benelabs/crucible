import React, { useMemo, useState } from 'react';
import {
  Area,
  AreaChart,
  Brush,
  CartesianGrid,
  Line,
  LineChart,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';
import { Activity, BarChart3, Cpu, LineChart as LineChartIcon, TrendingUp } from 'lucide-react';
import './GasTrendChart.css';

export type Timeframe = '24h' | '7d' | '30d';

export type GasMetric = 'gasCost' | 'cpuInstructions' | 'invocations';

export type ChartKind = 'area' | 'line';

/** A single raw observation emitted by the network indexer. */
export interface GasSample {
  timestamp: number;
  gasCost: number;
  cpuInstructions: number;
  invocations: number;
}

/** Raw samples collapsed into one fixed-width bucket of a timeframe. */
export interface AggregatedPoint {
  bucket: number;
  label: string;
  gasCost: number;
  cpuInstructions: number;
  invocations: number;
  sampleCount: number;
}

export interface MetricStats {
  min: number;
  max: number;
  average: number;
  p95: number;
}

const HOUR = 60 * 60 * 1000;
const DAY = 24 * HOUR;

export const TIMEFRAMES: Record<Timeframe, { label: string; windowMs: number; bucketMs: number }> = {
  '24h': { label: '24 hours', windowMs: DAY, bucketMs: HOUR },
  '7d': { label: '7 days', windowMs: 7 * DAY, bucketMs: 6 * HOUR },
  '30d': { label: '30 days', windowMs: 30 * DAY, bucketMs: DAY },
};

export const METRICS: Record<GasMetric, { label: string; unit: string; color: string }> = {
  gasCost: { label: 'Average Gas Cost', unit: 'stroops', color: '#8b5cf6' },
  cpuInstructions: { label: 'CPU Instructions', unit: 'instr', color: '#06b6d4' },
  invocations: { label: 'Invocations', unit: 'calls', color: '#f59e0b' },
};

/**
 * Invocations accumulate over a bucket while cost metrics are averaged, so the
 * aggregation strategy is metric specific.
 */
const SUMMED_METRICS: GasMetric[] = ['invocations'];

/** Linear-interpolated percentile, matching the "inclusive" definition used by most spreadsheets. */
export function percentile(values: number[], fraction: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const position = (sorted.length - 1) * fraction;
  const lower = Math.floor(position);
  const upper = Math.ceil(position);
  if (lower === upper) return sorted[lower];
  return sorted[lower] + (sorted[upper] - sorted[lower]) * (position - lower);
}

export function computeStats(values: number[]): MetricStats {
  if (values.length === 0) {
    return { min: 0, max: 0, average: 0, p95: 0 };
  }
  const total = values.reduce((sum, value) => sum + value, 0);
  return {
    min: Math.min(...values),
    max: Math.max(...values),
    average: total / values.length,
    p95: percentile(values, 0.95),
  };
}

export function formatBucketLabel(timestamp: number, timeframe: Timeframe): string {
  const date = new Date(timestamp);
  if (timeframe === '24h') {
    return `${String(date.getUTCHours()).padStart(2, '0')}:00`;
  }
  const day = String(date.getUTCDate()).padStart(2, '0');
  const month = String(date.getUTCMonth() + 1).padStart(2, '0');
  if (timeframe === '7d') {
    return `${month}/${day} ${String(date.getUTCHours()).padStart(2, '0')}h`;
  }
  return `${month}/${day}`;
}

/**
 * Collapses raw samples into the fixed buckets of the given timeframe.
 * Buckets without samples are omitted; the result is ordered oldest first.
 */
export function aggregateSamples(samples: GasSample[], timeframe: Timeframe): AggregatedPoint[] {
  const { bucketMs } = TIMEFRAMES[timeframe];
  const buckets = new Map<number, GasSample[]>();

  for (const sample of samples) {
    const bucket = Math.floor(sample.timestamp / bucketMs) * bucketMs;
    const existing = buckets.get(bucket);
    if (existing) {
      existing.push(sample);
    } else {
      buckets.set(bucket, [sample]);
    }
  }

  return [...buckets.entries()]
    .sort(([a], [b]) => a - b)
    .map(([bucket, bucketSamples]) => {
      const reduce = (metric: GasMetric) => {
        const total = bucketSamples.reduce((sum, sample) => sum + sample[metric], 0);
        return SUMMED_METRICS.includes(metric) ? total : total / bucketSamples.length;
      };
      return {
        bucket,
        label: formatBucketLabel(bucket, timeframe),
        gasCost: Math.round(reduce('gasCost')),
        cpuInstructions: Math.round(reduce('cpuInstructions')),
        invocations: Math.round(reduce('invocations')),
        sampleCount: bucketSamples.length,
      };
    });
}

/**
 * Deterministic sample generator standing in for the indexer feed, so the chart
 * renders identical history for a given `now` across reloads and tests.
 */
export function generateSamples(timeframe: Timeframe, now: number): GasSample[] {
  const { windowMs, bucketMs } = TIMEFRAMES[timeframe];
  const step = bucketMs / 4;
  const start = now - windowMs;
  const samples: GasSample[] = [];

  for (let timestamp = start; timestamp <= now; timestamp += step) {
    const index = Math.round((timestamp - start) / step);
    // Deterministic pseudo-noise plus a periodic fee spike every 17th sample.
    const wave = Math.sin(index / 3.5) * 220 + Math.cos(index / 11) * 140;
    const spike = index % 17 === 0 ? 1900 : 0;
    samples.push({
      timestamp,
      gasCost: Math.round(1450 + wave + spike),
      cpuInstructions: Math.round(920_000 + wave * 380 + spike * 210),
      invocations: Math.max(1, Math.round(38 + Math.sin(index / 5) * 12 + (spike > 0 ? 24 : 0))),
    });
  }

  return samples;
}

export function formatMetricValue(value: number, metric: GasMetric): string {
  if (metric === 'cpuInstructions' && Math.abs(value) >= 1000) {
    return `${(value / 1_000_000).toFixed(2)}M`;
  }
  return Math.round(value).toLocaleString('en-US');
}

export interface GasTrendChartProps {
  /** Overrides the generated feed; primarily used by tests and embedding views. */
  samples?: GasSample[];
  /** Fixed clock used to build the deterministic sample window. */
  now?: number;
}

export const GasTrendChart: React.FC<GasTrendChartProps> = ({ samples, now }) => {
  const [timeframe, setTimeframe] = useState<Timeframe>('24h');
  const [metric, setMetric] = useState<GasMetric>('gasCost');
  const [chartKind, setChartKind] = useState<ChartKind>('area');

  const referenceNow = useMemo(() => now ?? Date.UTC(2026, 7, 29, 12, 0, 0), [now]);

  const data = useMemo(
    () => aggregateSamples(samples ?? generateSamples(timeframe, referenceNow), timeframe),
    [samples, timeframe, referenceNow],
  );

  const stats = useMemo(() => computeStats(data.map((point) => point[metric])), [data, metric]);

  const { color, unit, label } = METRICS[metric];
  const ChartComponent = chartKind === 'area' ? AreaChart : LineChart;

  return (
    <div className="gas-trend-container" data-testid="gas-trend-chart">
      <div className="gas-trend-header">
        <div className="gas-trend-icon-wrapper">
          <TrendingUp className="gas-trend-icon" />
        </div>
        <div>
          <h2>Historical Gas Trends</h2>
          <p>Average gas cost, CPU instructions and invocation volume over time</p>
        </div>
      </div>

      <div className="gas-trend-toolbar glass-panel">
        <div className="gas-trend-control-group" role="group" aria-label="Timeframe">
          {(Object.keys(TIMEFRAMES) as Timeframe[]).map((key) => (
            <button
              key={key}
              type="button"
              className={`gas-trend-chip ${timeframe === key ? 'active' : ''}`}
              onClick={() => setTimeframe(key)}
              aria-pressed={timeframe === key}
              data-testid={`timeframe-${key}`}
            >
              {key}
            </button>
          ))}
        </div>

        <div className="gas-trend-control-group" role="group" aria-label="Metric">
          {(Object.keys(METRICS) as GasMetric[]).map((key) => (
            <button
              key={key}
              type="button"
              className={`gas-trend-chip ${metric === key ? 'active' : ''}`}
              onClick={() => setMetric(key)}
              aria-pressed={metric === key}
              data-testid={`metric-${key}`}
            >
              {key === 'gasCost' && <Activity size={14} />}
              {key === 'cpuInstructions' && <Cpu size={14} />}
              {key === 'invocations' && <BarChart3 size={14} />}
              {METRICS[key].label}
            </button>
          ))}
        </div>

        <div className="gas-trend-control-group" role="group" aria-label="Chart type">
          <button
            type="button"
            className={`gas-trend-chip ${chartKind === 'area' ? 'active' : ''}`}
            onClick={() => setChartKind('area')}
            aria-pressed={chartKind === 'area'}
            data-testid="chart-kind-area"
          >
            Area
          </button>
          <button
            type="button"
            className={`gas-trend-chip ${chartKind === 'line' ? 'active' : ''}`}
            onClick={() => setChartKind('line')}
            aria-pressed={chartKind === 'line'}
            data-testid="chart-kind-line"
          >
            <LineChartIcon size={14} />
            Line
          </button>
        </div>
      </div>

      <div className="gas-trend-stats">
        {(['min', 'average', 'p95', 'max'] as const).map((key) => (
          <div className="gas-trend-stat-card glass-panel" key={key}>
            <span className="gas-trend-stat-label">{key === 'p95' ? '95th percentile' : key}</span>
            <span className="gas-trend-stat-value" data-testid={`stat-${key}`}>
              {formatMetricValue(stats[key], metric)}
            </span>
            <span className="gas-trend-stat-unit">{unit}</span>
          </div>
        ))}
      </div>

      <div className="gas-trend-chart glass-panel" data-testid="gas-trend-canvas">
        <h3 className="gas-trend-chart-title">
          {label} · last {TIMEFRAMES[timeframe].label}
        </h3>
        <ResponsiveContainer width="100%" height={320}>
          <ChartComponent data={data} margin={{ top: 10, right: 16, left: 0, bottom: 0 }}>
            <defs>
              <linearGradient id="gasTrendFill" x1="0" y1="0" x2="0" y2="1">
                <stop offset="5%" stopColor={color} stopOpacity={0.35} />
                <stop offset="95%" stopColor={color} stopOpacity={0} />
              </linearGradient>
            </defs>
            <CartesianGrid strokeDasharray="3 3" stroke="#334155" vertical={false} />
            <XAxis dataKey="label" stroke="#94a3b8" fontSize={12} tickLine={false} axisLine={false} />
            <YAxis
              stroke="#94a3b8"
              fontSize={12}
              tickLine={false}
              axisLine={false}
              tickFormatter={(value) => formatMetricValue(Number(value), metric)}
            />
            <Tooltip
              contentStyle={{
                backgroundColor: 'rgba(15, 23, 42, 0.9)',
                borderColor: '#334155',
                borderRadius: '8px',
              }}
              itemStyle={{ color: '#e2e8f0' }}
              formatter={(value) => [`${formatMetricValue(Number(value), metric)} ${unit}`, label]}
            />
            <ReferenceLine y={stats.min} stroke="#22c55e" strokeDasharray="4 4" label={{ value: 'min', fill: '#22c55e', fontSize: 11 }} />
            <ReferenceLine y={stats.average} stroke="#94a3b8" strokeDasharray="4 4" label={{ value: 'avg', fill: '#94a3b8', fontSize: 11 }} />
            <ReferenceLine y={stats.p95} stroke="#f59e0b" strokeDasharray="4 4" label={{ value: 'p95', fill: '#f59e0b', fontSize: 11 }} />
            <ReferenceLine y={stats.max} stroke="#ef4444" strokeDasharray="4 4" label={{ value: 'max', fill: '#ef4444', fontSize: 11 }} />
            {chartKind === 'area' ? (
              <Area type="monotone" dataKey={metric} stroke={color} strokeWidth={2} fillOpacity={1} fill="url(#gasTrendFill)" name={label} />
            ) : (
              <Line type="monotone" dataKey={metric} stroke={color} strokeWidth={2} dot={false} name={label} />
            )}
            <Brush dataKey="label" height={24} stroke={color} travellerWidth={8} fill="rgba(15, 23, 42, 0.6)" />
          </ChartComponent>
        </ResponsiveContainer>
      </div>
    </div>
  );
};
