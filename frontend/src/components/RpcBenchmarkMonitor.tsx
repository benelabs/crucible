import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Activity, Gauge, Pause, Play, RefreshCw, Server, Wifi } from 'lucide-react';
import './RpcBenchmarkMonitor.css';

export const PING_INTERVAL_MS = 10_000;
/** Number of recent pings kept per node when averaging latency. */
export const LATENCY_WINDOW = 12;
/** Average latency above which a node is considered degraded. */
export const DEGRADED_LATENCY_MS = 750;
/** Ledger sequences a node may trail consensus by before it counts as lagging. */
export const MAX_LEDGER_DRIFT = 2;

export interface RpcNode {
  id: string;
  name: string;
  url: string;
  network: string;
}

export interface PingResult {
  ok: boolean;
  latencyMs: number;
  ledgerSequence: number | null;
  error?: string;
}

export type NodeStatus = 'pending' | 'healthy' | 'degraded' | 'lagging' | 'offline';

/** The rolling measurement retained per node between ping cycles. */
export interface NodeMeasurement {
  samples: number[];
  latest: PingResult | null;
  averageLatency: number;
  ledgerSequence: number | null;
  drift: number;
  status: NodeStatus;
}

export interface NodeState extends NodeMeasurement {
  node: RpcNode;
}

export const STATUS_BADGES: Record<NodeStatus, { label: string; modifier: string }> = {
  pending: { label: 'Checking', modifier: 'pending' },
  healthy: { label: 'Healthy', modifier: 'healthy' },
  degraded: { label: 'Degraded', modifier: 'degraded' },
  lagging: { label: 'Lagging', modifier: 'lagging' },
  offline: { label: 'Offline', modifier: 'offline' },
};

export const DEFAULT_NODES: RpcNode[] = [
  { id: 'soroban-mainnet', name: 'Stellar Public RPC', url: 'https://mainnet.sorobanrpc.com', network: 'pubnet' },
  { id: 'soroban-testnet', name: 'Stellar Testnet RPC', url: 'https://soroban-testnet.stellar.org', network: 'testnet' },
  { id: 'soroban-futurenet', name: 'Futurenet RPC', url: 'https://rpc-futurenet.stellar.org', network: 'futurenet' },
];

/** Rolling mean of the retained latency samples, rounded to whole milliseconds. */
export function computeAverageLatency(samples: number[]): number {
  if (samples.length === 0) return 0;
  const total = samples.reduce((sum, sample) => sum + sample, 0);
  return Math.round(total / samples.length);
}

/** Consensus is the highest ledger sequence any reachable node reported. */
export function computeConsensusLedger(sequences: (number | null)[]): number | null {
  const reported = sequences.filter((sequence): sequence is number => typeof sequence === 'number');
  return reported.length === 0 ? null : Math.max(...reported);
}

/** How many ledgers a node trails consensus by; never negative. */
export function computeDrift(sequence: number | null, consensus: number | null): number {
  if (sequence === null || consensus === null) return 0;
  return Math.max(0, consensus - sequence);
}

export function resolveNodeStatus(input: {
  latest: PingResult | null;
  averageLatency: number;
  drift: number;
}): NodeStatus {
  if (input.latest === null) return 'pending';
  if (!input.latest.ok) return 'offline';
  if (input.drift > MAX_LEDGER_DRIFT) return 'lagging';
  if (input.averageLatency > DEGRADED_LATENCY_MS) return 'degraded';
  return 'healthy';
}

export function appendSample(samples: number[], latencyMs: number): number[] {
  return [...samples, latencyMs].slice(-LATENCY_WINDOW);
}

/**
 * Issues a `getLatestLedger` JSON-RPC call and measures round-trip latency.
 * Never throws: transport failures come back as an unreachable result.
 */
export async function pingNode(
  node: RpcNode,
  fetchImpl: typeof fetch = fetch,
  clock: () => number = () => Date.now(),
): Promise<PingResult> {
  const startedAt = clock();
  try {
    const response = await fetchImpl(node.url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: node.id, method: 'getLatestLedger' }),
    });
    const latencyMs = Math.max(0, Math.round(clock() - startedAt));

    if (!response.ok) {
      return { ok: false, latencyMs, ledgerSequence: null, error: `HTTP ${response.status}` };
    }

    const payload = await response.json();
    const sequence = payload?.result?.sequence;
    return {
      ok: true,
      latencyMs,
      ledgerSequence: typeof sequence === 'number' ? sequence : null,
    };
  } catch (error) {
    return {
      ok: false,
      latencyMs: Math.max(0, Math.round(clock() - startedAt)),
      ledgerSequence: null,
      error: error instanceof Error ? error.message : 'Unreachable',
    };
  }
}

export interface RpcBenchmarkMonitorProps {
  nodes?: RpcNode[];
  fetchImpl?: typeof fetch;
  /** Disables the 10s polling loop; the manual refresh still works. */
  autoStart?: boolean;
}

const PENDING_MEASUREMENT: NodeMeasurement = {
  samples: [],
  latest: null,
  averageLatency: 0,
  ledgerSequence: null,
  drift: 0,
  status: 'pending',
};

export const RpcBenchmarkMonitor: React.FC<RpcBenchmarkMonitorProps> = ({
  nodes = DEFAULT_NODES,
  fetchImpl,
  autoStart = true,
}) => {
  const [measurements, setMeasurements] = useState<Record<string, NodeMeasurement>>({});
  const [running, setRunning] = useState(autoStart);
  const [lastCheckedAt, setLastCheckedAt] = useState<string | null>(null);
  const inFlight = useRef(false);

  // Identity of the configured fleet, so an inline `nodes` array does not
  // restart the polling loop on every render.
  const nodesKey = nodes.map((node) => `${node.id}:${node.url}`).join('|');

  const nodesRef = useRef(nodes);
  const fetchRef = useRef(fetchImpl);
  useEffect(() => {
    nodesRef.current = nodes;
    fetchRef.current = fetchImpl;
  });

  const runPingCycle = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      const current = nodesRef.current;
      const results = await Promise.all(current.map((node) => pingNode(node, fetchRef.current ?? fetch)));
      const consensus = computeConsensusLedger(results.map((result) => result.ledgerSequence));

      setMeasurements((previous) => {
        const next: Record<string, NodeMeasurement> = { ...previous };
        current.forEach((node, index) => {
          const result = results[index];
          const prior = previous[node.id];
          const samples = result.ok ? appendSample(prior?.samples ?? [], result.latencyMs) : (prior?.samples ?? []);
          const averageLatency = computeAverageLatency(samples);
          const drift = computeDrift(result.ledgerSequence, consensus);

          next[node.id] = {
            samples,
            latest: result,
            averageLatency,
            ledgerSequence: result.ledgerSequence,
            drift,
            status: resolveNodeStatus({ latest: result, averageLatency, drift }),
          };
        });
        return next;
      });
      setLastCheckedAt(new Date().toISOString());
    } finally {
      inFlight.current = false;
    }
  }, []);

  useEffect(() => {
    if (!running) return undefined;
    void runPingCycle();
    const timer = setInterval(() => {
      void runPingCycle();
    }, PING_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [running, runPingCycle, nodesKey]);

  // Nodes that have never answered stay in the pending state until their first ping.
  const states = useMemo<NodeState[]>(
    () => nodes.map((node) => ({ node, ...(measurements[node.id] ?? PENDING_MEASUREMENT) })),
    [nodes, measurements],
  );

  const consensusLedger = useMemo(
    () => computeConsensusLedger(states.map((state) => state.ledgerSequence)),
    [states],
  );

  const fleetAverage = useMemo(
    () => computeAverageLatency(states.flatMap((state) => state.samples)),
    [states],
  );

  return (
    <div className="rpc-monitor-container" data-testid="rpc-benchmark-monitor">
      <div className="rpc-monitor-header">
        <div className="rpc-monitor-icon-wrapper">
          <Server className="rpc-monitor-icon" />
        </div>
        <div>
          <h2>RPC Benchmark Monitor</h2>
          <p>Latency, ledger sync and availability across configured Soroban RPC nodes</p>
        </div>
      </div>

      <div className="rpc-monitor-toolbar glass-panel">
        <div className="rpc-monitor-summary">
          <div className="rpc-monitor-summary-item">
            <Gauge size={14} />
            <span className="rpc-monitor-summary-label">Fleet latency</span>
            <span className="rpc-monitor-summary-value" data-testid="fleet-latency">
              {fleetAverage} ms
            </span>
          </div>
          <div className="rpc-monitor-summary-item">
            <Activity size={14} />
            <span className="rpc-monitor-summary-label">Consensus ledger</span>
            <span className="rpc-monitor-summary-value" data-testid="consensus-ledger">
              {consensusLedger === null ? '—' : consensusLedger.toLocaleString('en-US')}
            </span>
          </div>
          <div className="rpc-monitor-summary-item">
            <Wifi size={14} />
            <span className="rpc-monitor-summary-label">Interval</span>
            <span className="rpc-monitor-summary-value">{PING_INTERVAL_MS / 1000}s</span>
          </div>
        </div>

        <div className="rpc-monitor-actions">
          <button
            type="button"
            className="rpc-monitor-btn"
            onClick={() => setRunning((value) => !value)}
            data-testid="toggle-polling"
          >
            {running ? <Pause size={14} /> : <Play size={14} />}
            {running ? 'Pause' : 'Resume'}
          </button>
          <button
            type="button"
            className="rpc-monitor-btn"
            onClick={() => void runPingCycle()}
            data-testid="refresh-now"
          >
            <RefreshCw size={14} />
            Ping now
          </button>
        </div>
      </div>

      <ul className="rpc-monitor-list">
        {states.map((state) => {
          const badge = STATUS_BADGES[state.status];
          return (
            <li className="rpc-monitor-card glass-panel" key={state.node.id} data-testid={`node-${state.node.id}`}>
              <div className="rpc-monitor-card-head">
                <div>
                  <h3 className="rpc-monitor-node-name">{state.node.name}</h3>
                  <p className="rpc-monitor-node-url">{state.node.url}</p>
                </div>
                <span
                  className={`rpc-monitor-badge rpc-monitor-badge--${badge.modifier}`}
                  data-testid={`status-${state.node.id}`}
                  data-status={state.status}
                >
                  {badge.label}
                </span>
              </div>

              <dl className="rpc-monitor-metrics">
                <div>
                  <dt>Avg latency</dt>
                  <dd data-testid={`latency-${state.node.id}`}>{state.averageLatency} ms</dd>
                </div>
                <div>
                  <dt>Ledger</dt>
                  <dd data-testid={`ledger-${state.node.id}`}>
                    {state.ledgerSequence === null ? '—' : state.ledgerSequence.toLocaleString('en-US')}
                  </dd>
                </div>
                <div>
                  <dt>Drift</dt>
                  <dd
                    className={state.drift > MAX_LEDGER_DRIFT ? 'rpc-monitor-drift--high' : undefined}
                    data-testid={`drift-${state.node.id}`}
                  >
                    {state.drift === 0 ? 'in sync' : `-${state.drift}`}
                  </dd>
                </div>
                <div>
                  <dt>Network</dt>
                  <dd>{state.node.network}</dd>
                </div>
              </dl>

              {state.latest?.error && (
                <p className="rpc-monitor-error" data-testid={`error-${state.node.id}`}>
                  {state.latest.error}
                </p>
              )}

              <div className="rpc-monitor-sparkline" aria-hidden="true">
                {state.samples.map((sample, index) => (
                  <span
                    key={`${state.node.id}-${index}`}
                    className="rpc-monitor-spark"
                    style={{ height: `${Math.min(100, Math.max(8, (sample / DEGRADED_LATENCY_MS) * 100))}%` }}
                  />
                ))}
              </div>
            </li>
          );
        })}
      </ul>

      <p className="rpc-monitor-footnote" data-testid="last-checked">
        {lastCheckedAt ? `Last checked ${lastCheckedAt}` : 'Awaiting first ping cycle'}
      </p>
    </div>
  );
};
