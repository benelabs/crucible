/**
 * Derives a state machine from a contract's interface and renders it two ways:
 * as Mermaid `stateDiagram-v2` source, and as a self-contained SVG.
 *
 * The SVG is laid out here rather than by a rendering library so the diagram
 * has no runtime dependency and can be asserted on directly in tests.
 */

export interface FlowchartTransition {
  /** Function that drives the transition. */
  name: string;
  from: string;
  to: string;
  /** Roles permitted to invoke it, shown on the edge. */
  requires?: string;
}

export interface ContractFlowchartSpec {
  name: string;
  initial: string;
  states: string[];
  /** States from which no further transition is possible. */
  terminal?: string[];
  transitions: FlowchartTransition[];
}

export interface FlowchartNode {
  id: string;
  label: string;
  x: number;
  y: number;
  width: number;
  height: number;
  isInitial: boolean;
  isTerminal: boolean;
}

export interface FlowchartEdge {
  id: string;
  label: string;
  from: string;
  to: string;
  /** True when this edge leaves the currently active state. */
  enabled: boolean;
  path: string;
  labelX: number;
  labelY: number;
  selfLoop: boolean;
}

export interface FlowchartLayout {
  nodes: FlowchartNode[];
  edges: FlowchartEdge[];
  width: number;
  height: number;
}

const NODE_WIDTH = 150;
const NODE_HEIGHT = 48;
const COLUMN_GAP = 92;
const ROW_GAP = 34;
const MARGIN = 24;
const MAX_PER_COLUMN = 4;

/** Escape text that will be interpolated into SVG markup. */
export function escapeXml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;');
}

/**
 * Mermaid state ids may not contain spaces or punctuation, so states are
 * emitted with a sanitised id and a quoted display label.
 */
export function toMermaidId(state: string): string {
  const id = state.replace(/[^A-Za-z0-9_]/g, '_');
  return /^[0-9]/.test(id) ? `s_${id}` : id;
}

/**
 * Build Mermaid `stateDiagram-v2` source. `activeState` is highlighted with a
 * class so the same source can be pasted into any Mermaid renderer.
 */
export function toMermaid(spec: ContractFlowchartSpec, activeState?: string): string {
  const lines: string[] = ['stateDiagram-v2'];

  lines.push(`    [*] --> ${toMermaidId(spec.initial)}`);

  spec.transitions.forEach((t) => {
    const label = t.requires ? `${t.name} (${t.requires})` : t.name;
    lines.push(`    ${toMermaidId(t.from)} --> ${toMermaidId(t.to)}: ${label}`);
  });

  (spec.terminal ?? []).forEach((state) => {
    lines.push(`    ${toMermaidId(state)} --> [*]`);
  });

  spec.states.forEach((state) => {
    const id = toMermaidId(state);
    if (id !== state) lines.push(`    ${id}: ${state}`);
  });

  if (activeState && spec.states.includes(activeState)) {
    lines.push('    classDef active fill:#0ea5e9,stroke:#7dd3fc,color:#0f172a,font-weight:bold');
    lines.push(`    class ${toMermaidId(activeState)} active`);
  }

  return lines.join('\n');
}

/**
 * Place states in columns, then route edges between them.
 *
 * Column assignment is by breadth-first distance from the initial state, which
 * keeps the common case — a contract that advances through stages — reading
 * left to right. States unreachable from the initial state are appended so they
 * remain visible rather than being silently dropped.
 */
export function layoutFlowchart(spec: ContractFlowchartSpec, activeState?: string): FlowchartLayout {
  const depth = new Map<string, number>();
  const queue: string[] = [];

  if (spec.states.includes(spec.initial)) {
    depth.set(spec.initial, 0);
    queue.push(spec.initial);
  }

  while (queue.length > 0) {
    const current = queue.shift() as string;
    const currentDepth = depth.get(current) ?? 0;
    spec.transitions
      .filter((t) => t.from === current && t.to !== current)
      .forEach((t) => {
        if (!depth.has(t.to) && spec.states.includes(t.to)) {
          depth.set(t.to, currentDepth + 1);
          queue.push(t.to);
        }
      });
  }

  let overflowColumn = Math.max(0, ...Array.from(depth.values(), (d) => d + 1));
  spec.states.forEach((state) => {
    if (!depth.has(state)) depth.set(state, overflowColumn);
  });
  overflowColumn = Math.max(...Array.from(depth.values()));

  // Group by column, splitting any column that grows past MAX_PER_COLUMN so
  // tall diagrams stay readable instead of running off the canvas.
  const columns = new Map<number, string[]>();
  spec.states.forEach((state) => {
    const column = depth.get(state) ?? 0;
    const bucket = columns.get(column) ?? [];
    bucket.push(state);
    columns.set(column, bucket);
  });

  const nodes: FlowchartNode[] = [];
  const sortedColumns = Array.from(columns.keys()).sort((a, b) => a - b);
  let x = MARGIN;
  let maxRows = 1;

  sortedColumns.forEach((column) => {
    const members = columns.get(column) ?? [];
    const chunks: string[][] = [];
    for (let i = 0; i < members.length; i += MAX_PER_COLUMN) {
      chunks.push(members.slice(i, i + MAX_PER_COLUMN));
    }
    chunks.forEach((chunk) => {
      chunk.forEach((state, row) => {
        nodes.push({
          id: state,
          label: state,
          x,
          y: MARGIN + row * (NODE_HEIGHT + ROW_GAP),
          width: NODE_WIDTH,
          height: NODE_HEIGHT,
          isInitial: state === spec.initial,
          isTerminal: (spec.terminal ?? []).includes(state),
        });
      });
      maxRows = Math.max(maxRows, chunk.length);
      x += NODE_WIDTH + COLUMN_GAP;
    });
  });

  const byId = new Map(nodes.map((n) => [n.id, n]));

  const edges: FlowchartEdge[] = spec.transitions
    .filter((t) => byId.has(t.from) && byId.has(t.to))
    .map((t, index) => {
      const from = byId.get(t.from) as FlowchartNode;
      const to = byId.get(t.to) as FlowchartNode;
      const enabled = activeState === undefined || t.from === activeState;
      const label = t.requires ? `${t.name} · ${t.requires}` : t.name;

      if (t.from === t.to) {
        // Self-transition: arc out of the top edge and back.
        const cx = from.x + from.width / 2;
        const top = from.y;
        return {
          id: `${t.name}-${index}`,
          label,
          from: t.from,
          to: t.to,
          enabled,
          path: `M ${cx - 18} ${top} C ${cx - 34} ${top - 40}, ${cx + 34} ${top - 40}, ${cx + 18} ${top}`,
          labelX: cx,
          labelY: top - 30,
          selfLoop: true,
        };
      }

      const forward = to.x >= from.x;
      const startX = forward ? from.x + from.width : from.x;
      const endX = forward ? to.x : to.x + to.width;
      const startY = from.y + from.height / 2;
      const endY = to.y + to.height / 2;
      const midX = (startX + endX) / 2;

      return {
        id: `${t.name}-${index}`,
        label,
        from: t.from,
        to: t.to,
        enabled,
        path: `M ${startX} ${startY} C ${midX} ${startY}, ${midX} ${endY}, ${endX} ${endY}`,
        labelX: midX,
        labelY: (startY + endY) / 2 - 8,
        selfLoop: false,
      };
    });

  return {
    nodes,
    edges,
    width: Math.max(x - COLUMN_GAP + MARGIN, NODE_WIDTH + MARGIN * 2),
    height: MARGIN * 2 + maxRows * NODE_HEIGHT + (maxRows - 1) * ROW_GAP + 48,
  };
}

/** Render a laid-out diagram as standalone SVG markup. */
export function toSvg(spec: ContractFlowchartSpec, activeState?: string): string {
  const layout = layoutFlowchart(spec, activeState);

  const edges = layout.edges
    .map((edge) => {
      const stroke = edge.enabled ? '#38bdf8' : '#334155';
      const marker = edge.enabled ? 'url(#arrow-active)' : 'url(#arrow)';
      return [
        `<path d="${edge.path}" fill="none" stroke="${stroke}" stroke-width="${edge.enabled ? 2 : 1.25}"`,
        ` marker-end="${marker}" data-edge="${escapeXml(edge.id)}"/>`,
        `<text x="${edge.labelX}" y="${edge.labelY}" text-anchor="middle" font-size="10"`,
        ` fill="${edge.enabled ? '#7dd3fc' : '#64748b'}">${escapeXml(edge.label)}</text>`,
      ].join('');
    })
    .join('\n  ');

  const nodes = layout.nodes
    .map((node) => {
      const active = node.id === activeState;
      const fill = active ? '#0ea5e9' : '#1e293b';
      const stroke = active ? '#7dd3fc' : node.isInitial ? '#22c55e' : '#334155';
      const text = active ? '#0f172a' : '#e2e8f0';
      return [
        `<g data-state="${escapeXml(node.id)}"${active ? ' data-active="true"' : ''}>`,
        `<rect x="${node.x}" y="${node.y}" width="${node.width}" height="${node.height}" rx="10"`,
        ` fill="${fill}" stroke="${stroke}" stroke-width="${active ? 2.5 : 1.5}"`,
        node.isTerminal ? ' stroke-dasharray="5 3"' : '',
        `/>`,
        `<text x="${node.x + node.width / 2}" y="${node.y + node.height / 2 + 4}" text-anchor="middle"`,
        ` font-size="12" font-weight="${active ? 700 : 500}" fill="${text}">${escapeXml(node.label)}</text>`,
        `</g>`,
      ].join('');
    })
    .join('\n  ');

  return [
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${layout.width} ${layout.height}"`,
    ` width="100%" role="img" aria-label="${escapeXml(spec.name)} state machine">`,
    `\n  <defs>`,
    `<marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">`,
    `<path d="M 0 0 L 10 5 L 0 10 z" fill="#334155"/></marker>`,
    `<marker id="arrow-active" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">`,
    `<path d="M 0 0 L 10 5 L 0 10 z" fill="#38bdf8"/></marker>`,
    `</defs>\n  `,
    edges,
    `\n  `,
    nodes,
    `\n</svg>`,
  ].join('');
}

/** Transitions available from a given state. */
export function permittedTransitions(
  spec: ContractFlowchartSpec,
  state: string,
): FlowchartTransition[] {
  return spec.transitions.filter((t) => t.from === state);
}

/**
 * Infer a state machine from a plain function list when the ABI carries no
 * explicit transition metadata.
 *
 * Soroban contracts conventionally name lifecycle functions after the stage
 * they open (`initialize`, `fund`, `close`), so each recognised verb advances
 * the machine one stage and everything else becomes a self-transition on the
 * stage it is callable from.
 */
export function inferFlowchart(
  contractName: string,
  functions: Array<{ name: string }>,
): ContractFlowchartSpec {
  const stageVerbs: Array<{ match: RegExp; state: string }> = [
    { match: /^(initialize|init|create|new)/, state: 'Initialized' },
    { match: /^(fund|deposit|stake|lock)/, state: 'Funded' },
    { match: /^(open|start|activate|publish)/, state: 'Active' },
    { match: /^(vote|bid|submit|propose)/, state: 'InProgress' },
    { match: /^(settle|release|execute|finalize|complete)/, state: 'Settled' },
    { match: /^(cancel|abort|refund|dispute)/, state: 'Cancelled' },
    { match: /^(close|destroy|terminate)/, state: 'Closed' },
  ];

  const states: string[] = ['Uninitialized'];
  const transitions: FlowchartTransition[] = [];
  let current = 'Uninitialized';

  functions.forEach((fn) => {
    const stage = stageVerbs.find((s) => s.match.test(fn.name));
    if (stage) {
      if (!states.includes(stage.state)) states.push(stage.state);
      transitions.push({ name: fn.name, from: current, to: stage.state });
      // Cancellation is an exit, not the new baseline for later stages.
      if (stage.state !== 'Cancelled') current = stage.state;
    } else {
      transitions.push({ name: fn.name, from: current, to: current });
    }
  });

  const terminal = states.filter((s) => s === 'Settled' || s === 'Closed' || s === 'Cancelled');

  return { name: contractName, initial: 'Uninitialized', states, terminal, transitions };
}
