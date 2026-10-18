import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import {
  GasTrendChart,
  TIMEFRAMES,
  aggregateSamples,
  computeStats,
  formatBucketLabel,
  formatMetricValue,
  generateSamples,
  percentile,
  type GasSample,
} from './GasTrendChart';

// Recharts needs a measurable container, which jsdom does not provide.
vi.mock('recharts', async () => {
  const actual = (await vi.importActual('recharts')) as Record<string, unknown>;
  return {
    ...actual,
    ResponsiveContainer: ({ children }: any) => <div>{children}</div>,
    AreaChart: ({ children }: any) => <div data-testid="area-chart">{children}</div>,
    LineChart: ({ children }: any) => <div data-testid="line-chart">{children}</div>,
  };
});

const HOUR = 60 * 60 * 1000;
const BASE = Date.UTC(2026, 7, 29, 0, 0, 0);

const sampleAt = (offsetMs: number, overrides: Partial<GasSample> = {}): GasSample => ({
  timestamp: BASE + offsetMs,
  gasCost: 1000,
  cpuInstructions: 500_000,
  invocations: 10,
  ...overrides,
});

describe('percentile', () => {
  it('returns 0 for an empty series', () => {
    expect(percentile([], 0.95)).toBe(0);
  });

  it('returns the exact value when the position lands on an index', () => {
    expect(percentile([10, 20, 30, 40, 50], 0.5)).toBe(30);
    expect(percentile([10, 20, 30, 40, 50], 1)).toBe(50);
  });

  it('interpolates between neighbouring values', () => {
    // position = (4 - 1) * 0.95 = 2.85 -> 30 + (40 - 30) * 0.85
    expect(percentile([10, 20, 30, 40], 0.95)).toBeCloseTo(38.5, 10);
  });

  it('sorts the input before selecting', () => {
    expect(percentile([50, 10, 40, 20, 30], 0.5)).toBe(30);
  });
});

describe('computeStats', () => {
  it('returns zeroed stats for an empty series', () => {
    expect(computeStats([])).toEqual({ min: 0, max: 0, average: 0, p95: 0 });
  });

  it('computes min, max, average and the 95th percentile', () => {
    const stats = computeStats([100, 200, 300, 400, 900]);

    expect(stats.min).toBe(100);
    expect(stats.max).toBe(900);
    expect(stats.average).toBe(380);
    // position = 4 * 0.95 = 3.8 -> 400 + (900 - 400) * 0.8
    expect(stats.p95).toBeCloseTo(800, 10);
  });
});

describe('aggregateSamples', () => {
  it('averages cost metrics and sums invocations inside a bucket', () => {
    const points = aggregateSamples(
      [
        sampleAt(0, { gasCost: 1000, cpuInstructions: 400_000, invocations: 5 }),
        sampleAt(15 * 60 * 1000, { gasCost: 2000, cpuInstructions: 600_000, invocations: 7 }),
      ],
      '24h',
    );

    expect(points).toHaveLength(1);
    expect(points[0]).toMatchObject({
      gasCost: 1500,
      cpuInstructions: 500_000,
      invocations: 12,
      sampleCount: 2,
    });
  });

  it('splits samples across buckets and orders them oldest first', () => {
    const points = aggregateSamples(
      [
        sampleAt(2 * HOUR, { gasCost: 3000 }),
        sampleAt(0, { gasCost: 1000 }),
        sampleAt(HOUR, { gasCost: 2000 }),
      ],
      '24h',
    );

    expect(points.map((point) => point.gasCost)).toEqual([1000, 2000, 3000]);
    expect(points.map((point) => point.bucket)).toEqual([BASE, BASE + HOUR, BASE + 2 * HOUR]);
  });

  it('uses the bucket width of the selected timeframe', () => {
    const samples = [sampleAt(0), sampleAt(5 * HOUR), sampleAt(7 * HOUR)];

    // 24h buckets hourly -> three points; 7d buckets every 6h -> two points.
    expect(aggregateSamples(samples, '24h')).toHaveLength(3);
    expect(aggregateSamples(samples, '7d')).toHaveLength(2);
    expect(aggregateSamples(samples, '30d')).toHaveLength(1);
  });

  it('returns an empty series when there are no samples', () => {
    expect(aggregateSamples([], '7d')).toEqual([]);
  });
});

describe('formatBucketLabel', () => {
  it('formats each timeframe at the right granularity', () => {
    const timestamp = Date.UTC(2026, 7, 29, 14, 0, 0);

    expect(formatBucketLabel(timestamp, '24h')).toBe('14:00');
    expect(formatBucketLabel(timestamp, '7d')).toBe('08/29 14h');
    expect(formatBucketLabel(timestamp, '30d')).toBe('08/29');
  });
});

describe('formatMetricValue', () => {
  it('abbreviates CPU instructions to millions', () => {
    expect(formatMetricValue(1_250_000, 'cpuInstructions')).toBe('1.25M');
  });

  it('renders small values and other metrics as grouped integers', () => {
    expect(formatMetricValue(950, 'cpuInstructions')).toBe('950');
    expect(formatMetricValue(12_500, 'gasCost')).toBe('12,500');
  });
});

describe('generateSamples', () => {
  it('is deterministic for a fixed clock', () => {
    expect(generateSamples('24h', BASE)).toEqual(generateSamples('24h', BASE));
  });

  it('covers the requested window', () => {
    const samples = generateSamples('7d', BASE);

    expect(samples[0].timestamp).toBe(BASE - TIMEFRAMES['7d'].windowMs);
    expect(samples[samples.length - 1].timestamp).toBeLessThanOrEqual(BASE);
    expect(samples.every((sample) => sample.invocations >= 1)).toBe(true);
  });
});

describe('GasTrendChart', () => {
  it('renders the default 24h area chart', () => {
    render(<GasTrendChart now={BASE} />);

    expect(screen.getByText('Historical Gas Trends')).toBeInTheDocument();
    expect(screen.getByTestId('area-chart')).toBeInTheDocument();
    expect(screen.getByTestId('timeframe-24h')).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByText(/Average Gas Cost · last 24 hours/)).toBeInTheDocument();
  });

  it('switches timeframe when a selector is clicked', () => {
    render(<GasTrendChart now={BASE} />);

    fireEvent.click(screen.getByTestId('timeframe-30d'));

    expect(screen.getByTestId('timeframe-30d')).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByTestId('timeframe-24h')).toHaveAttribute('aria-pressed', 'false');
    expect(screen.getByText(/last 30 days/)).toBeInTheDocument();
  });

  it('switches the rendered chart type', () => {
    render(<GasTrendChart now={BASE} />);

    fireEvent.click(screen.getByTestId('chart-kind-line'));

    expect(screen.getByTestId('line-chart')).toBeInTheDocument();
    expect(screen.queryByTestId('area-chart')).not.toBeInTheDocument();
  });

  it('shows reference statistics for the selected metric', () => {
    const samples: GasSample[] = [
      sampleAt(0, { gasCost: 1000, invocations: 4 }),
      sampleAt(HOUR, { gasCost: 3000, invocations: 6 }),
    ];

    render(<GasTrendChart samples={samples} now={BASE} />);

    expect(screen.getByTestId('stat-min')).toHaveTextContent('1,000');
    expect(screen.getByTestId('stat-max')).toHaveTextContent('3,000');
    expect(screen.getByTestId('stat-average')).toHaveTextContent('2,000');

    fireEvent.click(screen.getByTestId('metric-invocations'));

    expect(screen.getByTestId('stat-min')).toHaveTextContent('4');
    expect(screen.getByTestId('stat-max')).toHaveTextContent('6');
  });
});
