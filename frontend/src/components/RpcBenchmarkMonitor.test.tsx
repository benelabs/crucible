import { render, screen, waitFor, act, fireEvent } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DEGRADED_LATENCY_MS,
  LATENCY_WINDOW,
  MAX_LEDGER_DRIFT,
  PING_INTERVAL_MS,
  RpcBenchmarkMonitor,
  STATUS_BADGES,
  appendSample,
  computeAverageLatency,
  computeConsensusLedger,
  computeDrift,
  pingNode,
  resolveNodeStatus,
  type PingResult,
  type RpcNode,
} from './RpcBenchmarkMonitor';

const NODES: RpcNode[] = [
  { id: 'alpha', name: 'Alpha RPC', url: 'https://alpha.example', network: 'testnet' },
  { id: 'beta', name: 'Beta RPC', url: 'https://beta.example', network: 'testnet' },
];

const ok = (overrides: Partial<PingResult> = {}): PingResult => ({
  ok: true,
  latencyMs: 100,
  ledgerSequence: 1000,
  ...overrides,
});

/** Builds a fetch stub that answers each node URL with a fixed ledger sequence. */
const fetchStub = (byUrl: Record<string, { sequence?: number; status?: number; reject?: boolean }>) =>
  vi.fn(async (url: string) => {
    const entry = byUrl[url];
    if (!entry || entry.reject) {
      throw new Error('Connection refused');
    }
    return {
      ok: (entry.status ?? 200) < 400,
      status: entry.status ?? 200,
      json: async () => ({ jsonrpc: '2.0', result: { sequence: entry.sequence } }),
    };
  }) as unknown as typeof fetch;

describe('computeAverageLatency', () => {
  it('returns 0 without samples', () => {
    expect(computeAverageLatency([])).toBe(0);
  });

  it('rounds the mean of the retained samples', () => {
    expect(computeAverageLatency([100, 200, 301])).toBe(200);
  });
});

describe('appendSample', () => {
  it('keeps only the most recent window of samples', () => {
    let samples: number[] = [];
    for (let index = 0; index < LATENCY_WINDOW + 5; index += 1) {
      samples = appendSample(samples, index);
    }

    expect(samples).toHaveLength(LATENCY_WINDOW);
    expect(samples[samples.length - 1]).toBe(LATENCY_WINDOW + 4);
  });
});

describe('computeConsensusLedger', () => {
  it('is the highest sequence reported by a reachable node', () => {
    expect(computeConsensusLedger([1000, null, 1004])).toBe(1004);
  });

  it('is null when no node reported a sequence', () => {
    expect(computeConsensusLedger([null, null])).toBeNull();
  });
});

describe('computeDrift', () => {
  it('measures how far a node trails consensus', () => {
    expect(computeDrift(998, 1004)).toBe(6);
  });

  it('never reports negative drift or drift without data', () => {
    expect(computeDrift(1010, 1004)).toBe(0);
    expect(computeDrift(null, 1004)).toBe(0);
    expect(computeDrift(1000, null)).toBe(0);
  });
});

describe('resolveNodeStatus', () => {
  it('is pending before the first ping', () => {
    expect(resolveNodeStatus({ latest: null, averageLatency: 0, drift: 0 })).toBe('pending');
  });

  it('is offline when the node is unreachable', () => {
    expect(
      resolveNodeStatus({ latest: ok({ ok: false }), averageLatency: 20, drift: 0 }),
    ).toBe('offline');
  });

  it('is lagging when drift exceeds the allowed ledgers', () => {
    expect(
      resolveNodeStatus({ latest: ok(), averageLatency: 20, drift: MAX_LEDGER_DRIFT + 1 }),
    ).toBe('lagging');
  });

  it('is degraded when average latency exceeds the threshold', () => {
    expect(
      resolveNodeStatus({ latest: ok(), averageLatency: DEGRADED_LATENCY_MS + 1, drift: 0 }),
    ).toBe('degraded');
  });

  it('is healthy when in sync and responsive', () => {
    expect(
      resolveNodeStatus({ latest: ok(), averageLatency: 120, drift: MAX_LEDGER_DRIFT }),
    ).toBe('healthy');
  });
});

describe('pingNode', () => {
  it('measures latency and reads the ledger sequence', async () => {
    const clock = vi.fn().mockReturnValueOnce(1_000).mockReturnValueOnce(1_180);
    const fetchImpl = fetchStub({ 'https://alpha.example': { sequence: 4242 } });

    const result = await pingNode(NODES[0], fetchImpl, clock);

    expect(result).toEqual({ ok: true, latencyMs: 180, ledgerSequence: 4242 });
    expect(fetchImpl).toHaveBeenCalledWith(
      'https://alpha.example',
      expect.objectContaining({ method: 'POST' }),
    );
  });

  it('reports a null sequence when the payload omits it', async () => {
    const fetchImpl = fetchStub({ 'https://alpha.example': {} });

    await expect(pingNode(NODES[0], fetchImpl, () => 0)).resolves.toMatchObject({
      ok: true,
      ledgerSequence: null,
    });
  });

  it('reports HTTP errors without throwing', async () => {
    const fetchImpl = fetchStub({ 'https://alpha.example': { status: 503 } });

    await expect(pingNode(NODES[0], fetchImpl, () => 0)).resolves.toMatchObject({
      ok: false,
      error: 'HTTP 503',
    });
  });

  it('reports transport failures without throwing', async () => {
    const fetchImpl = fetchStub({});

    await expect(pingNode(NODES[0], fetchImpl, () => 0)).resolves.toMatchObject({
      ok: false,
      ledgerSequence: null,
      error: 'Connection refused',
    });
  });
});

describe('RpcBenchmarkMonitor', () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders a pending badge for every node before the first cycle', () => {
    render(<RpcBenchmarkMonitor nodes={NODES} fetchImpl={fetchStub({})} autoStart={false} />);

    expect(screen.getByTestId('status-alpha')).toHaveTextContent(STATUS_BADGES.pending.label);
    expect(screen.getByTestId('status-beta')).toHaveTextContent(STATUS_BADGES.pending.label);
    expect(screen.getByTestId('last-checked')).toHaveTextContent('Awaiting first ping cycle');
  });

  it('renders a healthy badge for the node at consensus and a lagging badge for the one behind', async () => {
    const fetchImpl = fetchStub({
      'https://alpha.example': { sequence: 1010 },
      'https://beta.example': { sequence: 1000 },
    });

    render(<RpcBenchmarkMonitor nodes={NODES} fetchImpl={fetchImpl} />);

    await waitFor(() => {
      expect(screen.getByTestId('status-alpha')).toHaveAttribute('data-status', 'healthy');
    });

    expect(screen.getByTestId('status-beta')).toHaveAttribute('data-status', 'lagging');
    expect(screen.getByTestId('status-beta')).toHaveTextContent(STATUS_BADGES.lagging.label);
    expect(screen.getByTestId('drift-beta')).toHaveTextContent('-10');
    expect(screen.getByTestId('drift-alpha')).toHaveTextContent('in sync');
    expect(screen.getByTestId('consensus-ledger')).toHaveTextContent('1,010');
  });

  it('renders an offline badge and the transport error for an unreachable node', async () => {
    const fetchImpl = fetchStub({
      'https://alpha.example': { sequence: 1000 },
      'https://beta.example': { reject: true },
    });

    render(<RpcBenchmarkMonitor nodes={NODES} fetchImpl={fetchImpl} />);

    await waitFor(() => {
      expect(screen.getByTestId('status-beta')).toHaveAttribute('data-status', 'offline');
    });

    expect(screen.getByTestId('error-beta')).toHaveTextContent('Connection refused');
    expect(screen.getByTestId('ledger-beta')).toHaveTextContent('—');
  });

  it('re-pings every interval while running and stops once paused', async () => {
    const fetchImpl = fetchStub({
      'https://alpha.example': { sequence: 1000 },
      'https://beta.example': { sequence: 1000 },
    });

    render(<RpcBenchmarkMonitor nodes={NODES} fetchImpl={fetchImpl} />);

    await waitFor(() => expect(fetchImpl).toHaveBeenCalledTimes(2));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(PING_INTERVAL_MS);
    });
    expect(fetchImpl).toHaveBeenCalledTimes(4);

    fireEvent.click(screen.getByTestId('toggle-polling'));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(PING_INTERVAL_MS * 2);
    });

    expect(fetchImpl).toHaveBeenCalledTimes(4);
    expect(screen.getByTestId('toggle-polling')).toHaveTextContent('Resume');
  });

  it('does not restart polling when re-rendered with an equivalent inline node list', async () => {
    const fetchImpl = fetchStub({
      'https://alpha.example': { sequence: 1000 },
      'https://beta.example': { sequence: 1000 },
    });
    // A fresh array literal each render must not retrigger the ping cycle.
    const view = render(<RpcBenchmarkMonitor nodes={[...NODES]} fetchImpl={fetchImpl} />);

    await waitFor(() => expect(fetchImpl).toHaveBeenCalledTimes(2));

    view.rerender(<RpcBenchmarkMonitor nodes={[...NODES]} fetchImpl={fetchImpl} />);
    view.rerender(<RpcBenchmarkMonitor nodes={[...NODES]} fetchImpl={fetchImpl} />);

    expect(fetchImpl).toHaveBeenCalledTimes(2);
    expect(screen.getByTestId('status-alpha')).toHaveAttribute('data-status', 'healthy');
  });

  it('pings on demand when polling is paused', async () => {
    const fetchImpl = fetchStub({
      'https://alpha.example': { sequence: 1000 },
      'https://beta.example': { sequence: 1000 },
    });

    render(<RpcBenchmarkMonitor nodes={NODES} fetchImpl={fetchImpl} autoStart={false} />);
    expect(fetchImpl).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('refresh-now'));

    await waitFor(() => {
      expect(screen.getByTestId('status-alpha')).toHaveAttribute('data-status', 'healthy');
    });
    expect(screen.getByTestId('last-checked')).toHaveTextContent('Last checked');
  });
});
