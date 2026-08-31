export type Severity = 'none' | 'low' | 'medium' | 'high' | 'critical';
export type UpdateStatus = 'up-to-date' | 'outdated' | 'deprecated';

export interface DepNode {
  id: string;
  name: string;
  version: string;
  severity: Severity;
  updateStatus: UpdateStatus;
  vulnerabilities: number;
  description: string;
}

export interface DepEdge {
  from: string;
  to: string;
}

export interface DepGraphData {
  nodes: DepNode[];
  edges: DepEdge[];
}

export interface NodePosition {
  x: number;
  y: number;
}

export const SEVERITY_COLORS: Record<Severity, string> = {
  none: '#3fb950',
  low: '#9be36b',
  medium: '#d29922',
  high: '#f0883e',
  critical: '#f85149',
};

export const WIDTH = 800;
export const HEIGHT = 520;
const CENTER = { x: WIDTH / 2, y: HEIGHT / 2 };

// Curated multi-crate dependency hierarchy used to audit circular references
// and vulnerable sub-crates (issue #910).
export const SAMPLE_DEP_GRAPH: DepGraphData = {
  nodes: [
    {
      id: 'crucible',
      name: 'crucible',
      version: '1.0.0',
      severity: 'none',
      updateStatus: 'up-to-date',
      vulnerabilities: 0,
      description: 'Batteries-included testing toolkit for Soroban smart contracts.',
    },
    {
      id: 'soroban-sdk',
      name: 'soroban-sdk',
      version: '25.0.0',
      severity: 'medium',
      updateStatus: 'outdated',
      vulnerabilities: 2,
      description: 'Rust SDK for writing Soroban smart contracts.',
    },
    {
      id: 'soroban-env-host',
      name: 'soroban-env-host',
      version: '24.0.1',
      severity: 'high',
      updateStatus: 'outdated',
      vulnerabilities: 3,
      description: 'Host environment used to execute Soroban contracts locally.',
    },
    {
      id: 'stellar-xdr',
      name: 'stellar-xdr',
      version: '22.0.0',
      severity: 'none',
      updateStatus: 'up-to-date',
      vulnerabilities: 0,
      description: 'Stellar XDR type definitions.',
    },
    {
      id: 'ed25519-dalek',
      name: 'ed25519-dalek',
      version: '1.0.1',
      severity: 'critical',
      updateStatus: 'deprecated',
      vulnerabilities: 5,
      description: 'Ed25519 signing/verification (deprecated, known RNG weakness).',
    },
    {
      id: 'sha2',
      name: 'sha2',
      version: '0.10.8',
      severity: 'none',
      updateStatus: 'up-to-date',
      vulnerabilities: 0,
      description: 'SHA-2 family hash functions.',
    },
    {
      id: 'serde',
      name: 'serde',
      version: '1.0.210',
      severity: 'low',
      updateStatus: 'up-to-date',
      vulnerabilities: 1,
      description: 'Serialization framework for Rust.',
    },
    {
      id: 'tokio',
      name: 'tokio',
      version: '1.38.0',
      severity: 'none',
      updateStatus: 'outdated',
      vulnerabilities: 0,
      description: 'Async runtime used by the Crucible backend.',
    },
  ],
  edges: [
    { from: 'crucible', to: 'soroban-sdk' },
    { from: 'crucible', to: 'soroban-env-host' },
    { from: 'crucible', to: 'serde' },
    { from: 'crucible', to: 'tokio' },
    { from: 'soroban-sdk', to: 'stellar-xdr' },
    { from: 'soroban-sdk', to: 'ed25519-dalek' },
    { from: 'soroban-sdk', to: 'sha2' },
    { from: 'soroban-env-host', to: 'stellar-xdr' },
    { from: 'soroban-env-host', to: 'ed25519-dalek' },
    { from: 'ed25519-dalek', to: 'sha2' },
    { from: 'serde', to: 'serde' },
  ],
};

// Tiny deterministic PRNG (mulberry32) so the layout is stable across renders
// and in tests.
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * Deterministic force-directed layout. Runs a fixed number of ticks so the
 * result is reproducible (no animation timers required for tests).
 */
export function computeLayout(
  data: DepGraphData,
  seed = 1337,
  ticks = 400,
): Record<string, NodePosition> {
  const rng = mulberry32(seed);
  const ids = data.nodes.map((n) => n.id);
  const pos: Record<string, NodePosition> = {};
  const angleStep = (Math.PI * 2) / Math.max(ids.length, 1);
  ids.forEach((id, i) => {
    const r = 160 + rng() * 60;
    pos[id] = {
      x: CENTER.x + Math.cos(angleStep * i) * r + (rng() - 0.5) * 30,
      y: CENTER.y + Math.sin(angleStep * i) * r + (rng() - 0.5) * 30,
    };
  });

  const repulsion = 9000;
  const spring = 0.02;
  const restLength = 130;
  const centerPull = 0.012;

  for (let t = 0; t < ticks; t++) {
    const disp: Record<string, NodePosition> = {};
    ids.forEach((id) => (disp[id] = { x: 0, y: 0 }));

    // Repulsion between every pair.
    for (let i = 0; i < ids.length; i++) {
      for (let j = i + 1; j < ids.length; j++) {
        const a = pos[ids[i]];
        const b = pos[ids[j]];
        const dx = a.x - b.x;
        const dy = a.y - b.y;
        const dist = Math.sqrt(dx * dx + dy * dy) || 0.01;
        const force = repulsion / (dist * dist);
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        disp[ids[i]].x += fx;
        disp[ids[i]].y += fy;
        disp[ids[j]].x -= fx;
        disp[ids[j]].y -= fy;
      }
    }

    // Spring attraction along edges.
    for (const e of data.edges) {
      const a = pos[e.from];
      const b = pos[e.to];
      if (!a || !b) continue;
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const dist = Math.sqrt(dx * dx + dy * dy) || 0.01;
      const force = (dist - restLength) * spring;
      const fx = (dx / dist) * force;
      const fy = (dy / dist) * force;
      disp[e.from].x += fx;
      disp[e.from].y += fy;
      disp[e.to].x -= fx;
      disp[e.to].y -= fy;
    }

    // Centering + integrate.
    ids.forEach((id) => {
      disp[id].x += (CENTER.x - pos[id].x) * centerPull;
      disp[id].y += (CENTER.y - pos[id].y) * centerPull;
      pos[id].x += Math.max(-12, Math.min(12, disp[id].x));
      pos[id].y += Math.max(-12, Math.min(12, disp[id].y));
      pos[id].x = Math.max(40, Math.min(WIDTH - 40, pos[id].x));
      pos[id].y = Math.max(40, Math.min(HEIGHT - 40, pos[id].y));
    });
  }

  return pos;
}

/** Detect cycles in the dependency graph (directed DFS). */
export function detectCycles(data: DepGraphData): boolean {
  const adj: Record<string, string[]> = {};
  data.nodes.forEach((n) => (adj[n.id] = []));
  data.edges.forEach((e) => {
    if (adj[e.from]) adj[e.from].push(e.to);
  });
  const WHITE = 0;
  const GRAY = 1;
  const BLACK = 2;
  const color: Record<string, number> = {};
  data.nodes.forEach((n) => (color[n.id] = WHITE));

  const visit = (u: string): boolean => {
    color[u] = GRAY;
    for (const v of adj[u] || []) {
      if (color[v] === GRAY) return true;
      if (color[v] === WHITE && visit(v)) return true;
    }
    color[u] = BLACK;
    return false;
  };

  for (const n of data.nodes) {
    if (color[n.id] === WHITE && visit(n.id)) return true;
  }
  return false;
}
